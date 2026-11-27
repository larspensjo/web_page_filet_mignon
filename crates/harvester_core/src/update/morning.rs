use super::archive;
use crate::morning_archive::{
    morning_block_reason, resolve_window, target_key, BatchArchiveOutcome,
    BatchArchiveReason as Reason, BatchArchiveStatus as Status, BriefingCheckpoint,
    CheckpointWrite, ExternalArchiveTarget, FailedStage, GateFacts, LeftOutArticle,
    PortfolioRecord, ResolvedWindow, TargetObservation,
};
use crate::{AppState, Effect, PipelineStage, StartupInputOutcome};
use chrono::{DateTime, Utc};
use engine_logging::{engine_info, engine_warn};
use harvester_engine::{archive_url_key, ArchiveWindow};
use std::{
    collections::{BTreeMap, HashMap},
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
    NothingNew(BatchArchiveOutcome),
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

    pub fn batch_archive_source_failures(&self) -> &[(String, String)] {
        self.run_progress()
            .map_or(&[], |run| run.source_failures.as_slice())
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
        (SavePurpose::NothingNew(mut result), saved) => {
            if !saved {
                result.status = Status::CycleFailed;
                result.reason = Some(Reason::RecordNotSaved);
                result.left_out.clear();
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
                result.left_out.clear();
                morning.outcome = Some(result);
            } else if !saved {
                result.status = Status::ExportFailed;
                result.reason = Some(Reason::RecordNotSaved);
                result.left_out.clear();
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
        left_out: Vec::new(),
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
        quota_halted: state.model_dispatch_halt_kind().is_some(),
        sources_scanned,
        failed_sources,
    };
    if let Some(reason) = morning_block_reason(facts) {
        engine_info!(
            "[morning-archive] target={} gate=blocked reason={}",
            morning.target.display(),
            reason.as_str()
        );
        state.morning_archive.as_mut().unwrap().outcome = Some(outcome(
            Status::CycleFailed,
            Some(reason),
            failed_sources,
            None,
        ));
        return Vec::new();
    }

    let mut record = resolved.record.clone();
    let mut left_out = Vec::new();
    let mut omissions = BTreeMap::new();
    for entry in state.saved_results_entries().filter(|entry| {
        entry.is_unsettled(state.briefing_triage_policy())
            && window.contains(entry.article.fetched_utc.as_deref())
    }) {
        let article = &entry.article;
        let key = archive_url_key(&article.url);
        if record.left_out.contains_key(&key) {
            continue;
        }
        let reason = match morning.failures.get(&key) {
            Some(stages) => format!(
                "failed:{}",
                stages
                    .iter()
                    .map(|stage| stage.as_str())
                    .collect::<Vec<_>>()
                    .join("+")
            ),
            None => "not_finished".to_owned(),
        };
        left_out.push(LeftOutArticle {
            url: article.url.clone(),
            title: article.source_title.clone(),
            reason,
        });
        let fetched_utc = article.fetched_utc.as_deref().and_then(|date| {
            DateTime::parse_from_rfc3339(date)
                .map(|fetched| fetched.with_timezone(&Utc))
                .ok()
        });
        omissions.insert(key, fetched_utc);
    }
    left_out.sort_unstable_by(|a, b| a.url.cmp(&b.url));

    let target = ExternalArchiveTarget {
        path: morning.target.clone(),
        disposition: resolved.disposition,
    };
    let prepared = archive::prepare_morning_request(state, request_id, target, window, &record);
    let Effect::ArchiveRequested { ordered_urls, .. } = &prepared else {
        unreachable!()
    };
    let selected = ordered_urls.len();
    engine_info!(
        "[morning-archive] target={} selected={selected} left_out={}",
        morning.target.display(),
        left_out.len()
    );
    for article in &left_out {
        engine_info!(
            "[morning-archive] left_out target={} url={} reason={}",
            morning.target.display(),
            article.url,
            article.reason
        );
    }
    if selected == 0 {
        let mut result = outcome(Status::NothingNew, None, failed_sources, None);
        result.left_out = left_out;
        if omissions.is_empty() {
            state.morning_archive.as_mut().unwrap().outcome = Some(result);
            return Vec::new();
        }
        record.left_out.append(&mut omissions);
        return vec![save_record(
            state,
            record,
            CheckpointWrite::Unchanged,
            SavePurpose::NothingNew(result),
        )];
    }

    let mut result = outcome(Status::Exported, None, failed_sources, Some(window));
    result.left_out = left_out;
    record.pending_until_utc = Some(window.until_utc());
    record.pending_left_out = omissions;
    state
        .morning_archive
        .as_mut()
        .unwrap()
        .window
        .as_mut()
        .unwrap()
        .record = record.clone();
    vec![save_record(
        state,
        record,
        CheckpointWrite::Unchanged,
        SavePurpose::Intent {
            request: Box::new(prepared),
            outcome: result,
        },
    )]
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
    record.complete_publication(window.until_utc());
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
    result.left_out.clear();
    result.previous_superseded = previous_kept.is_some() && previous_superseded;
    result.previous_kept = previous_kept;
    let target = morning.target.to_string_lossy().into_owned();
    engine_warn!("[morning-archive] target={target} request_id={request_id} publication failed reason={reason}");
    morning.outcome = Some(result);
    archive::handle_export_failed(state, request_id, target, reason.to_owned())
}
