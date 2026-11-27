use super::*;
use crate::morning_archive::*;
use chrono::{DateTime, Utc};
use harvester_engine::ArchiveWindow;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const A: &str = r"C:\Portfolio\Archive.md";
const B: &str = r"C:\Portfolio\Other.md";
const START: &str = "2026-10-03T06:00:00.123456Z";
const DESKTOP: &str = "2026-10-03T07:00:00Z";
const LAST: &str = "2026-10-03T10:00:00Z";
const PENDING: &str = "2026-10-03T11:00:00Z";

fn date(raw: &str) -> DateTime<Utc> {
    raw.parse().unwrap()
}

fn key(path: &str) -> String {
    target_key(Path::new(path))
}

fn persisted(target: &str, record: PortfolioRecord) -> BriefingCheckpoint {
    BriefingCheckpoint {
        since_utc: Some(DESKTOP.into()),
        portfolio: [(key(target), record)].into(),
    }
}

fn record(start: &str, last: Option<&str>, pending: Option<&str>) -> PortfolioRecord {
    PortfolioRecord {
        confirmed_until_utc: Some(date(start)),
        last_archive_until_utc: last.map(date),
        pending_until_utc: pending.map(date),
        ..Default::default()
    }
}

fn sidecar(until: &str) -> SidecarObservation {
    SidecarObservation::Parsed(Some(date(until)))
}

fn observation(sidecar: SidecarObservation, index: Option<ArchiveIndex>) -> TargetObservation {
    TargetObservation {
        sidecar,
        archive: index.map_or(ObservedArchive::Absent, |index| ObservedArchive::Present {
            stamp: Some(ArchiveFileStamp {
                len: 42,
                modified: UNIX_EPOCH,
            }),
            index,
        }),
    }
}

fn windowed(since: &str, until: &str) -> ArchiveIndex {
    ArchiveIndex::Schema2 {
        window_since_utc: Some(date(since)),
        window_until_utc: Some(date(until)),
        fetched_from: None,
        fetched_to: None,
    }
}

#[test]
fn sidecar_seeds_new_target_without_last_publication() {
    let resolved = resolve_window(
        &BriefingCheckpoint::default(),
        &key(A),
        &observation(sidecar(START), None),
        now(),
    )
    .unwrap();
    assert_eq!(resolved.record, record(START, None, None));
    assert_eq!(resolved.since_utc, date(START));
}

#[test]
fn checkpoint_seeds_new_target_without_sidecar() {
    let checkpoint = BriefingCheckpoint {
        since_utc: Some(START.into()),
        ..Default::default()
    };
    let resolved = resolve_window(
        &checkpoint,
        &key(A),
        &observation(SidecarObservation::Missing, None),
        now(),
    )
    .unwrap();
    assert_eq!(resolved.record, record(START, None, None));
}

#[test]
fn no_seed_reports_no_window_start() {
    assert_eq!(
        resolve_window(
            &BriefingCheckpoint::default(),
            &key(A),
            &observation(SidecarObservation::Missing, None),
            now()
        ),
        Err(BatchArchiveReason::NoWindowStart)
    );
}

#[test]
fn later_sidecar_advances_confirmed_start() {
    let resolved = resolve_window(
        &persisted(A, record(START, Some(LAST), None)),
        &key(A),
        &observation(sidecar(DESKTOP), None),
        now(),
    )
    .unwrap();
    assert_eq!(resolved.record, record(DESKTOP, Some(LAST), None));
    assert_eq!(resolved.since_utc, date(DESKTOP));
    assert_eq!(resolved.checkpoint_advance, None);
}

fn assert_sidecar_ignored(sidecar: SidecarObservation, last: Option<&str>, reason: SidecarStatus) {
    let durable = record(START, last, None);
    let resolved = resolve_window(
        &persisted(A, durable.clone()),
        &key(A),
        &observation(sidecar, None),
        now(),
    )
    .unwrap();
    assert_eq!(resolved.record, durable);
    assert_eq!(resolved.since_utc, date(START));
    assert_eq!(resolved.sidecar_status, reason);
}

#[test]
fn null_sidecar_preserves_record() {
    assert_sidecar_ignored(
        SidecarObservation::Parsed(None),
        Some(LAST),
        SidecarStatus::Null,
    );
}

#[test]
fn older_sidecar_preserves_record() {
    assert_sidecar_ignored(
        sidecar("2026-10-03T06:00:00Z"),
        Some(LAST),
        SidecarStatus::Older,
    );
}

#[test]
fn equal_sidecar_preserves_record() {
    assert_sidecar_ignored(
        sidecar("2026-10-03T07:00:00.123456+01:00"),
        Some(LAST),
        SidecarStatus::Equal,
    );
}

#[test]
fn missing_sidecar_preserves_record() {
    assert_sidecar_ignored(
        SidecarObservation::Missing,
        Some(LAST),
        SidecarStatus::Missing,
    );
}

#[test]
fn unreadable_sidecar_preserves_record() {
    assert_sidecar_ignored(
        SidecarObservation::Unreadable,
        Some(LAST),
        SidecarStatus::Unreadable,
    );
}

#[test]
fn unparseable_sidecar_preserves_record() {
    assert_sidecar_ignored(
        SidecarObservation::Unparseable,
        Some(LAST),
        SidecarStatus::Unparseable,
    );
}

#[test]
fn future_sidecar_preserves_record() {
    assert_sidecar_ignored(
        sidecar("2026-10-03T13:00:00Z"),
        Some("2026-10-04T06:00:00Z"),
        SidecarStatus::Future,
    );
}

#[test]
fn sidecar_beyond_last_archive_preserves_record() {
    assert_sidecar_ignored(
        sidecar(PENDING),
        Some(LAST),
        SidecarStatus::BeyondLastArchive,
    );
}

#[test]
fn seeded_target_without_last_publication_cannot_advance() {
    assert_sidecar_ignored(sidecar(LAST), None, SidecarStatus::NoLastArchive);
}

#[test]
fn reconcile_failed_checkpoint_save_before_advancing_from_sidecar() {
    let checkpoint = persisted(A, record(START, Some(LAST), Some(PENDING)));
    let seen = observation(sidecar(PENDING), Some(windowed(START, PENDING)));
    let resolved = resolve_window(&checkpoint, &key(A), &seen, now()).unwrap();
    assert_eq!(resolved.record, record(PENDING, Some(PENDING), None));
    assert_eq!(resolved.checkpoint_advance, Some(date(PENDING)));
}

#[test]
fn unpublished_intent_is_cleared_without_advancing_last_archive() {
    let checkpoint = persisted(A, record(START, Some(LAST), Some(PENDING)));
    let seen = observation(sidecar(PENDING), Some(windowed(START, LAST)));
    let resolved = resolve_window(&checkpoint, &key(A), &seen, now()).unwrap();
    assert_eq!(resolved.record, record(START, Some(LAST), None));
    assert_eq!(resolved.checkpoint_advance, None);
}

#[test]
fn crash_after_publication_recovers_even_before_consumer_confirmation() {
    let checkpoint = persisted(A, record(START, Some(LAST), Some(PENDING)));
    let seen = observation(SidecarObservation::Missing, Some(windowed(START, PENDING)));
    let resolved = resolve_window(&checkpoint, &key(A), &seen, now()).unwrap();
    assert_eq!(resolved.record, record(START, Some(PENDING), None));
    assert_eq!(resolved.checkpoint_advance, Some(date(PENDING)));
    assert_eq!(resolved.disposition, PreviousArchiveDisposition::KeepAside);
    let newer_checkpoint = BriefingCheckpoint {
        since_utc: Some(now().to_rfc3339()),
        ..checkpoint
    };
    assert_eq!(
        resolve_window(&newer_checkpoint, &key(A), &seen, now())
            .unwrap()
            .checkpoint_advance,
        None
    );
}

#[test]
fn export_a_then_b_then_a_keeps_a_record() {
    let mut checkpoint = persisted(A, record(START, Some(LAST), None));
    let a = observation(sidecar(LAST), Some(windowed(START, LAST)));
    let first = resolve_window(&checkpoint, &key(A), &a, now()).unwrap();
    checkpoint.portfolio.insert(key(A), first.record.clone());
    checkpoint.since_utc = Some(LAST.into());
    let b = resolve_window(
        &checkpoint,
        &key(B),
        &observation(SidecarObservation::Missing, None),
        now(),
    )
    .unwrap();
    assert_eq!(b.since_utc, date(LAST));
    checkpoint
        .portfolio
        .insert(key(B), record(LAST, Some(PENDING), None));
    checkpoint.since_utc = Some(PENDING.into());
    let again = resolve_window(&checkpoint, &key(r"c:\PORTFOLIO\archive.MD"), &a, now()).unwrap();
    assert_eq!(again.record, first.record);
}

fn set_up(
    state: AppState,
    checkpoint: BriefingCheckpoint,
    seen: TargetObservation,
    checkpoint_first: bool,
) -> (AppState, Vec<Effect>) {
    set_up_for(state, A, checkpoint, seen, checkpoint_first)
}

#[test]
fn configuring_after_the_checkpoint_reply_reloads_the_durable_record() {
    let fixture = Fixture::selected();
    let checkpoint = persisted(A, record(START, Some(LAST), None));
    let reply = Msg::BriefingCheckpointLoaded {
        since_utc: checkpoint.since_utc,
        portfolio: checkpoint.portfolio,
    };
    let state = update(metadata_replies(fixture.hydrate()), reply.clone()).0;
    assert_eq!(state.briefing_since_utc(), Some(date(DESKTOP)));
    let target = PathBuf::from(A);
    let (state, effects) = update(
        state,
        Msg::BatchArchiveConfigured {
            target: target.clone(),
        },
    );
    assert!(effects.contains(&Effect::LoadBriefingCheckpoint));
    no_load(&effects);
    let (state, effects) = update(
        state,
        Msg::ArchiveTargetObserved {
            target,
            observation: observation(SidecarObservation::Missing, None),
            observed_at: now(),
        },
    );
    assert!(!state.batch_archive_window_ready());
    assert!(effects.is_empty());
    let (state, effects) = update(state, reply);
    assert!(effects.is_empty());
    assert!(state.batch_archive_window_ready());
    load_window(state, &fixture.articles, START);
}

#[test]
fn morning_checkpoint_read_failure_stops_without_loading_polling_or_reseeding() {
    let fixture = Fixture::selected();
    let target = PathBuf::from(A);
    for error_first in [false, true] {
        let mut state = update(
            metadata_replies(fixture.hydrate()),
            Msg::BatchArchiveConfigured {
                target: target.clone(),
            },
        )
        .0;
        let error = Msg::BriefingCheckpointLoadFailed {
            reason: "sharing violation".into(),
        };
        let observed = Msg::ArchiveTargetObserved {
            target: target.clone(),
            observation: observation(sidecar(START), None),
            observed_at: now(),
        };
        let replies = if error_first {
            [error, observed]
        } else {
            [observed, error]
        };
        for message in replies.into_iter().chain([
            Msg::EvaluatePreTriageRefresh {
                ordered_urls: vec!["https://meter.example/a".into()],
                triggered_by_job_done: false,
            },
            Msg::PipelineRunRequested {
                scope: crate::PipelineRunScope::Full,
            },
            Msg::tick_at(now()),
        ]) {
            let (next, effects) = update(state, message);
            no_load(&effects);
            assert!(!effects
                .iter()
                .any(|effect| matches!(effect, Effect::SavePortfolioRecord { .. })));
            state = next;
        }
        assert!(!state.batch_archive_window_ready());
        assert_eq!(
            state.startup_inputs().checkpoint,
            crate::StartupInputOutcome::Failed
        );
        assert_outcome(
            &state,
            BatchArchiveStatus::CycleFailed,
            Some(BatchArchiveReason::RecordUnreadable),
        );
    }

    let desktop = update(
        AppState::new(),
        Msg::BriefingCheckpointLoadFailed {
            reason: "sharing violation".into(),
        },
    )
    .0;
    assert_eq!(desktop.briefing_since_utc(), None);
    assert_eq!(
        desktop.startup_inputs().checkpoint,
        crate::StartupInputOutcome::Loaded
    );
}

fn set_up_for(
    state: AppState,
    target: &str,
    checkpoint: BriefingCheckpoint,
    seen: TargetObservation,
    checkpoint_first: bool,
) -> (AppState, Vec<Effect>) {
    let target = PathBuf::from(target);
    let (state, effects) = update(
        state,
        Msg::BatchArchiveConfigured {
            target: target.clone(),
        },
    );
    assert_eq!(
        effects,
        [Effect::ObserveArchiveTarget {
            target: target.clone()
        }]
    );
    let checkpoint = Msg::BriefingCheckpointLoaded {
        since_utc: checkpoint.since_utc,
        portfolio: checkpoint.portfolio,
    };
    let observed = Msg::ArchiveTargetObserved {
        target,
        observation: seen,
        observed_at: now(),
    };
    let replies = if checkpoint_first {
        [checkpoint, observed]
    } else {
        [observed, checkpoint]
    };
    let [first, second] = replies;
    let (mut state, effects) = update(state, first);
    no_load(&effects);
    assert!(!state.batch_archive_window_ready());
    assert_eq!(
        state.startup_inputs().checkpoint,
        crate::StartupInputOutcome::Pending
    );
    let (next, effects) = update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: vec!["https://meter.example/before-ready".into()],
            triggered_by_job_done: false,
        },
    );
    no_load(&effects);
    state = next;
    for _ in 0..12 {
        let (next, effects) = update(state, Msg::tick_at(now()));
        no_load(&effects);
        state = next;
    }
    update(state, second)
}

fn no_load(effects: &[Effect]) {
    assert!(effects.iter().all(|e| !matches!(
        e,
        Effect::LoadArticlesForTriage { .. } | Effect::PollAllSources
    )));
}

fn save_id(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|e| match e {
            Effect::SavePortfolioRecord { save_id, .. } => Some(*save_id),
            _ => None,
        })
        .expect("record save must be requested")
}

fn load_window(mut state: AppState, articles: &[LoadedArticle], since: &str) -> AppState {
    state = update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: articles.iter().map(|a| a.url.clone()).collect(),
            triggered_by_job_done: false,
        },
    )
    .0;
    for _ in 0..40 {
        let (next, effects) = update(state, Msg::tick_at(now()));
        state = next;
        for effect in effects {
            if let Effect::LoadArticlesForTriage {
                request_id,
                since_utc,
                ..
            } = effect
            {
                assert_eq!(since_utc, Some(date(since)));
                return resolve(scan(state, request_id, articles), request_id, articles);
            }
        }
    }
    panic!("resolved window load did not dispatch");
}

fn metadata_replies(state: AppState) -> AppState {
    let state = update(state, metadata()).0;
    update(
        state,
        Msg::PromptContextsLoaded {
            contexts: HashMap::new(),
        },
    )
    .0
}

fn ready(
    f: &Fixture,
    checkpoint: BriefingCheckpoint,
    seen: TargetObservation,
    first: bool,
    expected: &str,
) -> AppState {
    ready_for(f, A, checkpoint, seen, first, expected)
}

fn ready_for(
    f: &Fixture,
    target: &str,
    checkpoint: BriefingCheckpoint,
    seen: TargetObservation,
    first: bool,
    expected: &str,
) -> AppState {
    let (state, effects) = set_up_for(
        metadata_replies(f.hydrate()),
        target,
        checkpoint,
        seen,
        first,
    );
    ready_after_setup(state, &effects, &f.articles, expected, |_| {})
}

fn ready_after_setup(
    mut state: AppState,
    startup_effects: &[Effect],
    articles: &[LoadedArticle],
    expected: &str,
    save: impl FnOnce(&[Effect]),
) -> AppState {
    no_load(startup_effects);
    if let Some(id) = startup_effects.iter().find_map(|e| match e {
        Effect::SavePortfolioRecord { save_id, .. } => Some(*save_id),
        _ => None,
    }) {
        assert!(!state.batch_archive_window_ready());
        assert_eq!(
            state.startup_inputs().checkpoint,
            crate::StartupInputOutcome::Pending
        );
        let (next, effects) = update(
            state,
            Msg::PortfolioRecordSaved {
                save_id: id + 1,
                result: Ok(()),
            },
        );
        no_load(&effects);
        assert!(!next.batch_archive_window_ready());
        state = next;
        save(startup_effects);
        state = update(
            state,
            Msg::PortfolioRecordSaved {
                save_id: id,
                result: Ok(()),
            },
        )
        .0;
    }
    assert!(state.batch_archive_window_ready());
    let state = load_window(state, articles, expected);
    assert_eq!(state.startup_readiness(), Readiness::Ready);
    state
}

fn request(state: AppState, submitted_at: DateTime<Utc>) -> (AppState, Vec<Effect>) {
    update(
        state,
        Msg::BatchArchiveRequested {
            request_id: 9,
            submitted_at,
        },
    )
}

fn acknowledge(state: AppState, effects: &[Effect]) -> (AppState, Vec<Effect>) {
    update(
        state,
        Msg::PortfolioRecordSaved {
            save_id: save_id(effects),
            result: Ok(()),
        },
    )
}

fn export(state: AppState, submitted_at: DateTime<Utc>) -> (AppState, Effect) {
    let (state, effects) = request(state, submitted_at);
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::ArchiveRequested { .. })));
    let (state, effects) = acknowledge(state, &effects);
    let effect = effects
        .into_iter()
        .find(|e| matches!(e, Effect::ArchiveRequested { .. }))
        .expect("intent acknowledgement should request publication");
    (state, effect)
}

fn startup_reply_order(checkpoint_first: bool) {
    let f = Fixture::selected();
    let state = ready(
        &f,
        persisted(A, record("2026-10-03T05:00:00Z", Some(LAST), None)),
        observation(sidecar(START), None),
        checkpoint_first,
        START,
    );
    let (run, effects) = update(
        state.clone(),
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    let (_, effects) =
        crate::fixture_support::complete_processing_configuration(run, effects, 100_000);
    let since: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::LoadArticlesForTriage { since_utc, .. } => Some(*since_utc),
            _ => None,
        })
        .collect();
    assert_eq!(since, [Some(date(START))]);
    let (_, effect) = export(state, now());
    assert!(
        matches!(effect, Effect::ArchiveRequested { window, .. } if window.since_utc() == Some(date(START)))
    );
}

#[test]
fn startup_checkpoint_reply_then_target_observation() {
    startup_reply_order(true);
}

#[test]
fn startup_target_observation_then_checkpoint_reply() {
    startup_reply_order(false);
}

#[test]
fn startup_record_save_failure_prevents_polls_loads_and_archive() {
    let f = Fixture::selected();
    let checkpoint = BriefingCheckpoint {
        since_utc: Some(START.into()),
        ..Default::default()
    };
    let (state, effects) = set_up(
        metadata_replies(f.hydrate()),
        checkpoint,
        observation(SidecarObservation::Missing, None),
        true,
    );
    let (state, effects) = update(
        state,
        Msg::PortfolioRecordSaved {
            save_id: save_id(&effects),
            result: Err("write refused".into()),
        },
    );
    no_load(&effects);
    assert!(!state.batch_archive_window_ready());
    assert_outcome(
        &state,
        BatchArchiveStatus::CycleFailed,
        Some(BatchArchiveReason::RecordNotSaved),
    );
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    assert!(effects.is_empty());
    assert!(request(state, now()).1.is_empty());
}

fn normal(f: &Fixture) -> AppState {
    ready(
        f,
        persisted(A, record(START, Some(LAST), None)),
        observation(SidecarObservation::Missing, None),
        true,
        START,
    )
}

fn assert_outcome(
    state: &AppState,
    status: BatchArchiveStatus,
    reason: Option<BatchArchiveReason>,
) {
    let outcome = state
        .batch_archive_outcome()
        .expect("terminal morning outcome");
    assert_eq!((outcome.status, outcome.reason), (status, reason));
}

fn assert_cycle_failed(state: AppState, reason: BatchArchiveReason) {
    let (state, effects) = request(state, now());
    assert!(effects.is_empty());
    assert_outcome(&state, BatchArchiveStatus::CycleFailed, Some(reason));
}

#[test]
fn gate_interrupt_precedes_all_other_reasons() {
    let state = update(normal(&Fixture::selected()), Msg::BatchArchiveInterrupted).0;
    let state = update(
        state,
        Msg::ResultStoreUnavailable {
            reason: "refused".into(),
        },
    )
    .0;
    assert_cycle_failed(state, BatchArchiveReason::Interrupted);
}

#[test]
fn gate_store_refusal_precedes_readiness() {
    let fixture = Fixture::selected();
    let (state, _) = set_up(
        metadata_replies(fixture.hydrate_with_store_failure(Some("refused"))),
        persisted(A, record(START, Some(LAST), None)),
        observation(SidecarObservation::Missing, None),
        true,
    );
    assert_ne!(state.startup_readiness(), Readiness::Ready);
    assert_cycle_failed(state, BatchArchiveReason::ResultStoreUnavailable);
}

#[test]
fn gate_requires_readiness() {
    let (state, _) = set_up(
        metadata_replies(Fixture::selected().hydrate()),
        persisted(A, record(START, Some(LAST), None)),
        observation(SidecarObservation::Missing, None),
        true,
    );
    assert_cycle_failed(state, BatchArchiveReason::SavedResultsUnavailable);
}

#[test]
fn gate_total_failure_is_based_on_current_source_intake() {
    let (mut state, effects) = update(
        normal(&Fixture::selected()),
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    assert!(effects.contains(&Effect::PollAllSources));
    state = update(state, Msg::PollStarted { total: 1 }).0;
    state = update(
        state,
        Msg::SourcePollFailed {
            source_id: harvester_engine::SourceId::new("bad").unwrap(),
            error: "failed".into(),
        },
    )
    .0;
    state = update(state, Msg::AllSourcesPollEnded).0;
    assert_cycle_failed(state, BatchArchiveReason::TotalFailure);
}

#[test]
fn gate_ignores_failed_downloads_with_no_download_successes() {
    let mut state = normal(&Fixture::selected());
    crate::update::pipeline_run::begin_run_if_needed(&mut state);
    let run = state.run_progress_mut().unwrap();
    run.stage_mut(crate::PipelineStage::ScanningSources).total = 1;
    run.stage_mut(crate::PipelineStage::ScanningSources)
        .completed = 1;
    run.stage_mut(crate::PipelineStage::DownloadingArticles)
        .failed = 3;
    let (_, effect) = export(state, now());
    assert!(matches!(
        effect,
        Effect::ArchiveRequested {
            use_summaries: true,
            ..
        }
    ));
}

#[test]
fn gate_ignores_whole_state_source_failures_from_earlier_runs() {
    let mut state = normal(&Fixture::selected());
    state.record_source_error(
        &harvester_engine::SourceId::new("bad").unwrap(),
        "prior run".into(),
    );
    export(state, now());
}

#[test]
fn gate_quota_halt_and_withdrawals_leave_backlog() {
    for quota_halt in [true, false] {
        let fixture = Fixture::new(&[("unfinished", Some(5), false, None)]);
        let mut state = normal(&fixture);
        if quota_halt {
            state.halt_model_dispatch(crate::state::ModelDispatchHalt::RunQuota("limit".into()));
        } else {
            state.briefing_mut().set_articles(fixture.articles.clone());
            assert_eq!(state.briefing_mut().withdraw_pending(), 1);
        }
        let (state, effects) = request(state, now());
        assert!(effects.is_empty());
        assert_outcome(
            &state,
            BatchArchiveStatus::CycleFailed,
            Some(BatchArchiveReason::Backlog),
        );
        assert_eq!(state.batch_archive_outcome().unwrap().backlog_articles, 1);
    }

    let fixture = Fixture::new(&[("withdrawn", Some(5), false, None)]);
    let mut durable = record(START, None, None);
    durable.strikes.insert(
        fixture.articles[0].url.clone(),
        Strike {
            count: 1,
            last_until: date(LAST),
        },
    );
    let state = ready(
        &fixture,
        persisted(A, durable),
        observation(SidecarObservation::Missing, None),
        true,
        START,
    );
    let state = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    )
    .0;
    let state = update(state, Msg::StopFinishClicked).0;
    let (state, effects) = request(state, now());
    assert!(effects.iter().any(|effect| matches!(effect, Effect::SavePortfolioRecord { record, .. } if record.strikes.is_empty())));
    let (state, _) = acknowledge(state, &effects);
    assert_outcome(
        &state,
        BatchArchiveStatus::CycleFailed,
        Some(BatchArchiveReason::Interrupted),
    );
}

#[test]
fn gate_zero_selected_does_not_fall_back_to_full_mode() {
    let f = Fixture::new(&[("below", Some(5), true, Some((1, "below")))]);
    let state = normal(&f);
    let (_, desktop) = update(state.clone(), Msg::ArchiveClicked);
    assert!(desktop.iter().any(|effect| matches!(
        effect,
        Effect::OpenArchiveDialog {
            signal_candidate_default: DialogDefault::OffEmpty,
            ..
        }
    )));
    let (state, effects) = request(state, now());
    assert!(effects.is_empty());
    assert_outcome(&state, BatchArchiveStatus::NothingNew, None);
    assert_eq!(state.briefing_since_utc(), Some(date(START)));
}

#[test]
fn gate_exports_signal_with_summaries_even_when_desktop_defaults_to_full() {
    let mut state = normal(&Fixture::selected());
    state
        .signal_candidate_mut()
        .enqueue("https://other.example/article".into(), "fixture".into());
    state
        .signal_candidate_mut()
        .mark_scoring("https://other.example/article", 987);
    let (_, desktop) = update(state.clone(), Msg::ArchiveClicked);
    assert!(desktop.iter().any(|effect| matches!(
        effect,
        Effect::OpenArchiveDialog {
            signal_candidate_default: DialogDefault::OffPartial,
            ..
        }
    )));
    let (_, effect) = export(state, now());
    assert!(
        matches!(effect, Effect::ArchiveRequested { external: Some(_), use_summaries: true, ordered_urls, summaries, requested_checkpoint, .. }
        if ordered_urls.len() == 2 && !summaries.is_empty() && requested_checkpoint == Some(now()))
    );
}

#[test]
fn desktop_and_batch_share_the_signal_selection_and_request_payload() {
    let state = normal(&Fixture::selected());
    let (desktop, _) = update(state.clone(), Msg::ArchiveClicked);
    let request_id = desktop.archive_request_id();
    let (_, desktop) = update(
        desktop,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".into(),
            set_checkpoint: true,
            submitted_at: now(),
            use_summaries: true,
            use_signal_candidates: true,
        },
    );
    let (_, batch) = export(state, now());
    fn parts(effect: Effect) -> impl PartialEq {
        let Effect::ArchiveRequested {
            ordered_urls,
            summaries,
            annotations,
            priority_snapshot,
            folds,
            ..
        } = effect
        else {
            panic!("expected export effect")
        };
        (
            ordered_urls,
            summaries,
            annotations,
            priority_snapshot,
            folds,
        )
    }
    assert!(
        parts(batch)
            == parts(
                desktop
                    .into_iter()
                    .find(|e| matches!(e, Effect::ArchiveRequested { .. }))
                    .unwrap()
            )
    );
}

fn completed(state: AppState) -> (AppState, Vec<Effect>) {
    update(
        state,
        Msg::BatchArchivePublished {
            request_id: 9,
            doc_count: 2,
            previous_kept: None,
            previous_superseded: false,
        },
    )
}

#[test]
fn stop_after_archive_requested_preserves_publication_outcome() {
    let (state, _) = export(normal(&Fixture::selected()), now());
    let state = update(state, Msg::StopFinishClicked).0;
    let state = update(state, Msg::BatchArchiveInterrupted).0;
    let (state, effects) = completed(state);
    let (state, _) = acknowledge(state, &effects);
    assert_outcome(&state, BatchArchiveStatus::Exported, None);
}

#[test]
fn intent_save_failure_prevents_publication() {
    let (state, effects) = request(normal(&Fixture::selected()), now());
    let (state, effects) = update(
        state,
        Msg::PortfolioRecordSaved {
            save_id: save_id(&effects),
            result: Err("intent refused".into()),
        },
    );
    assert!(effects.is_empty());
    assert_outcome(
        &state,
        BatchArchiveStatus::ExportFailed,
        Some(BatchArchiveReason::RecordNotSaved),
    );
}

#[test]
fn completion_acknowledgement_reports_exported() {
    let (state, _) = export(normal(&Fixture::selected()), now());
    let (state, effects) = completed(state);
    assert!(state.batch_archive_outcome().is_none());
    assert!(effects.iter().any(|e| matches!(e, Effect::SavePortfolioRecord { record, checkpoint: CheckpointWrite::Set(until), .. }
        if record.last_archive_until_utc == Some(now()) && record.pending_until_utc.is_none() && *until == now())));
    let (state, _) = acknowledge(state, &effects);
    assert_outcome(&state, BatchArchiveStatus::Exported, None);
    assert_eq!(state.briefing_since_utc(), Some(date(START)));
}

#[test]
fn completion_save_failure_reports_exported_with_checkpoint_not_saved() {
    let (state, _) = export(normal(&Fixture::selected()), now());
    let (state, effects) = completed(state);
    let (state, _) = update(
        state,
        Msg::PortfolioRecordSaved {
            save_id: save_id(&effects),
            result: Err("checkpoint refused".into()),
        },
    );
    assert_outcome(
        &state,
        BatchArchiveStatus::Exported,
        Some(BatchArchiveReason::CheckpointNotSaved),
    );
}

#[test]
fn worker_failure_reports_export_failed_without_checkpoint_save() {
    let (state, _) = export(normal(&Fixture::selected()), now());
    let (state, effects) = update(
        state,
        Msg::BatchArchivePublicationFailed {
            request_id: 9,
            reason: "rename".into(),
            previous_kept: None,
            previous_superseded: false,
        },
    );
    assert!(effects.is_empty());
    assert_outcome(
        &state,
        BatchArchiveStatus::ExportFailed,
        Some(BatchArchiveReason::ExportWriteFailed),
    );
}

#[test]
fn consecutive_real_failures_block_at_most_two_mornings() {
    let items = [BacklogArticle {
        url: "https://meter.example/a",
        unsettled: true,
        failed_stages: &[FailedStage::Scoring],
    }];
    let mut strikes = Default::default();
    for day in 10..=12 {
        let until = DateTime::from_timestamp(1_790_000_000 + day * 86_400, 0).unwrap();
        let gate = assess_backlog(&strikes, &items, until);
        if day < 12 {
            assert_eq!(gate.backlog_articles, 1);
            assert!(gate.excused.is_empty());
        } else {
            assert_eq!(gate.backlog_articles, 0);
            assert_eq!(
                gate.excused,
                [ExcusedArticle {
                    url: items[0].url.into(),
                    stages: vec![FailedStage::Scoring]
                }]
            );
        }
        strikes = gate.strikes;
    }
}

#[test]
fn mixed_quota_morning_records_actual_failure_and_resets_skipped_article() {
    let fixture = Fixture::new(&[
        ("genuine", Some(5), false, None),
        ("skipped", Some(5), false, None),
    ]);
    let mut durable = record(START, None, None);
    for article in &fixture.articles {
        durable.strikes.insert(
            article.url.clone(),
            Strike {
                count: 1,
                last_until: date(LAST),
            },
        );
    }
    let mut state = ready(
        &fixture,
        persisted(A, durable),
        observation(SidecarObservation::Missing, None),
        true,
        START,
    );
    state.briefing_mut().set_articles(fixture.articles.clone());
    for (index, request_id) in [41, 42].into_iter().enumerate() {
        state.briefing_mut().start_article(index, request_id);
        state.record_pending_llm_request(request_id, PromptId::ArticleSummary);
    }
    for (request_id, result) in [
        (
            41,
            crate::LlmResultKind::Failed {
                reason: "genuine failure".into(),
            },
        ),
        (
            42,
            crate::LlmResultKind::QuotaExhausted {
                reason: "not admitted by runtime budget".into(),
                origin: harvester_engine::llm::QuotaOrigin::SessionBudget,
            },
        ),
    ] {
        state = update(
            state,
            Msg::LlmCompleted {
                request_id,
                result,
                metadata: None,
            },
        )
        .0;
    }
    assert!(state.run_quota_halt_reason().is_some());
    let (state, effects) = request(state, date(PENDING));
    let saved = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SavePortfolioRecord { record, .. } => Some(record),
            _ => None,
        })
        .unwrap();
    assert_eq!(saved.strikes["https://meter.example/genuine"].count, 2);
    assert!(!saved.strikes.contains_key("https://meter.example/skipped"));
    let (state, _) = acknowledge(state, &effects);
    let outcome = state.batch_archive_outcome().unwrap();
    assert_eq!(outcome.backlog_articles, 2);
    assert_eq!(outcome.excused_articles, 0);
}

#[test]
fn success_between_failures_resets_strike_count() {
    let item = [BacklogArticle {
        url: "retry",
        unsettled: true,
        failed_stages: &[FailedStage::Summary],
    }];
    let first = assess_backlog(&Default::default(), &item, date(LAST));
    let success = assess_backlog(&first.strikes, &[], date(PENDING));
    let next = assess_backlog(&success.strikes, &item, now());
    assert_eq!(next.strikes["retry"].count, 1);
    assert_eq!(next.backlog_articles, 1);
}

fn morning_with_failure(
    fixture: &Fixture,
    checkpoint: BriefingCheckpoint,
    at: DateTime<Utc>,
    stage: FailedStage,
) -> (AppState, Vec<Effect>) {
    let mut state = ready(
        fixture,
        checkpoint,
        observation(SidecarObservation::Missing, None),
        true,
        START,
    );
    let retry = &fixture.articles[0];
    let request_id = 89;
    let prompt = match stage {
        FailedStage::Summary => {
            state.briefing_mut().set_articles(vec![retry.clone()]);
            state.briefing_mut().start_article(0, request_id);
            PromptId::ArticleSummary
        }
        FailedStage::Scoring => {
            state
                .signal_candidate_mut()
                .enqueue(retry.url.clone(), "score-attempt".into());
            state
                .signal_candidate_mut()
                .mark_scoring(&retry.url, request_id);
            PromptId::ArticleSignalCandidate
        }
        FailedStage::Triage => unreachable!(),
    };
    state.record_pending_llm_request(request_id, prompt);
    let state = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: crate::LlmResultKind::Failed {
                reason: "fixture failure".into(),
            },
            metadata: None,
        },
    )
    .0;
    request(state, at)
}

#[test]
fn excused_summary_is_omitted_from_request_when_a_current_score_selects_it() {
    let mut state = normal(&Fixture::selected());
    let url = "https://meter.example/a";
    let effect = crate::update::archive::prepare_morning_request(
        &mut state,
        9,
        ExternalArchiveTarget {
            path: A.into(),
            disposition: PreviousArchiveDisposition::None,
        },
        ArchiveWindow::from_submission(Some(date(START)), now()),
        &[ExcusedArticle {
            url: url.into(),
            stages: vec![FailedStage::Summary],
        }],
    );
    let Effect::ArchiveRequested {
        ordered_urls,
        summaries,
        ..
    } = effect
    else {
        unreachable!()
    };
    assert!(ordered_urls.iter().any(|candidate| candidate == url));
    assert!(!summaries.contains_key(url));
    assert!(!summaries.is_empty());
}

#[test]
fn gate_exports_excused_summary_failure_without_summary_when_current_score_survives() {
    let fixture = Fixture::selected();
    let article = &fixture.articles[0];
    let mut prior = record(START, Some(LAST), None);
    prior.strikes.insert(
        article.url.clone(),
        Strike {
            count: 2,
            last_until: date(PENDING),
        },
    );
    let mut state = ready(
        &fixture,
        persisted(A, prior),
        observation(SidecarObservation::Missing, None),
        true,
        START,
    );
    let retry_id = 93;
    state.briefing_mut().set_articles(vec![article.clone()]);
    state.briefing_mut().start_article(0, retry_id);
    state.record_pending_llm_request(retry_id, PromptId::ArticleSummary);
    let state = update(
        state,
        Msg::LlmCompleted {
            request_id: retry_id,
            result: crate::LlmResultKind::Failed {
                reason: "summary retry failed".into(),
            },
            metadata: None,
        },
    )
    .0;
    let current = state.saved_results_for_url(&article.url).unwrap();
    assert!(current.signal.is_some() && current.summary.is_some());
    let (state, pending) = request(state, now());
    assert!(pending.iter().any(|effect| matches!(effect,
        Effect::LogMorningArchiveExcusals { articles, .. } if articles.iter().any(|excused| excused.url == article.url)
    )));
    let (state, effects) = acknowledge(state, &pending);
    assert!(effects.iter().any(|effect| matches!(effect, Effect::ArchiveRequested { ordered_urls, summaries, .. }
        if ordered_urls.contains(&article.url) && !summaries.contains_key(&article.url) && !summaries.is_empty())));
    let (state, effects) = completed(state);
    let (state, _) = acknowledge(state, &effects);
    assert_eq!(state.batch_archive_outcome().unwrap().excused_articles, 1);
}

#[test]
fn blocked_morning_reports_every_failed_stage_for_the_excused_article() {
    let fixture = Fixture::new(&[
        ("retry", Some(5), false, None),
        ("other-backlog", Some(5), true, None),
    ]);
    let url = &fixture.articles[0].url;
    let mut prior = record(START, Some(LAST), None);
    prior.strikes.insert(
        url.clone(),
        Strike {
            count: 2,
            last_until: date(PENDING),
        },
    );
    let mut state = ready(
        &fixture,
        persisted(A, prior),
        observation(SidecarObservation::Missing, None),
        true,
        START,
    );
    let morning = state.morning_archive.as_mut().unwrap();
    morning.note_failure(url, FailedStage::Summary);
    morning.note_failure(url, FailedStage::Scoring);
    let (state, pending) = request(state, now());
    let expected = [ExcusedArticle {
        url: url.clone(),
        stages: vec![FailedStage::Summary, FailedStage::Scoring],
    }];
    assert!(pending.iter().any(|effect| matches!(effect, Effect::LogMorningArchiveExcusals { articles, blocked: true, .. } if articles == &expected)));
    let (state, effects) = acknowledge(state, &pending);
    assert!(effects.is_empty());
    assert_outcome(
        &state,
        BatchArchiveStatus::CycleFailed,
        Some(BatchArchiveReason::Backlog),
    );
    let result = state.batch_archive_outcome().unwrap();
    assert_eq!((result.excused_articles, result.backlog_articles), (1, 1));
    assert_eq!(result.excused, expected);
}

#[test]
fn failed_summary_is_reported_and_recovery_can_enter_only_an_unconfirmed_replay() {
    let failed = Fixture::new(&[
        ("retry", Some(5), false, None),
        ("other", Some(5), true, Some((95, "other"))),
    ]);
    let mut checkpoint = persisted(A, record(START, None, None));
    for day in [date(LAST), date(PENDING), now()] {
        let (state, effects) =
            morning_with_failure(&failed, checkpoint.clone(), day, FailedStage::Summary);
        let saved = effects
            .iter()
            .find_map(|e| match e {
                Effect::SavePortfolioRecord { record, .. } => Some(record),
                _ => None,
            })
            .unwrap();
        checkpoint.portfolio.insert(key(A), saved.clone());
        let (state, followup) = acknowledge(state, &effects);
        if day != now() {
            assert_outcome(
                &state,
                BatchArchiveStatus::CycleFailed,
                Some(BatchArchiveReason::Backlog),
            );
            continue;
        }
        assert!(effects.iter().any(|e| matches!(e, Effect::LogMorningArchiveExcusals { articles, blocked: false, .. } if articles == &[ExcusedArticle { url: "https://meter.example/retry".into(), stages: vec![FailedStage::Summary] }])));
        assert!(followup.iter().any(|e| matches!(e, Effect::ArchiveRequested { ordered_urls, .. } if ordered_urls == &["https://meter.example/other"])));
        let (state, saved) = completed(state);
        let (state, _) = acknowledge(state, &saved);
        assert_outcome(&state, BatchArchiveStatus::Exported, None);
        assert_eq!(state.batch_archive_outcome().unwrap().excused_articles, 1);
        checkpoint
            .portfolio
            .insert(key(A), record(START, Some("2026-10-03T12:00:00Z"), None));
    }
    let recovered = Fixture::new(&[
        ("retry", Some(5), true, Some((99, "retry"))),
        ("other", Some(5), true, Some((95, "other"))),
    ]);
    let state = ready(
        &recovered,
        checkpoint.clone(),
        observation(
            SidecarObservation::Parsed(None),
            Some(windowed(START, "2026-10-03T12:00:00Z")),
        ),
        false,
        START,
    );
    let (_, effect) = export(state, now() + chrono::TimeDelta::hours(1));
    assert!(
        matches!(effect, Effect::ArchiveRequested { ordered_urls, summaries, .. } if ordered_urls.contains(&"https://meter.example/retry".into()) && summaries.contains_key("https://meter.example/retry"))
    );
    let state = ready(
        &recovered,
        checkpoint,
        observation(
            sidecar("2026-10-03T12:00:00Z"),
            Some(windowed(START, "2026-10-03T12:00:00Z")),
        ),
        true,
        "2026-10-03T12:00:00Z",
    );
    let (state, effects) = request(state, now() + chrono::TimeDelta::hours(2));
    assert!(effects.is_empty());
    assert_outcome(&state, BatchArchiveStatus::NothingNew, None);
}

#[test]
fn scoring_failure_on_third_morning_exports_other_articles_and_logs_url() {
    let fixture = Fixture::new(&[
        ("retry", Some(5), true, None),
        ("other", Some(5), true, Some((95, "other"))),
    ]);
    let mut durable = record(START, None, None);
    for day in [date(LAST), date(PENDING), now()] {
        let (state, effects) =
            morning_with_failure(&fixture, persisted(A, durable), day, FailedStage::Scoring);
        durable = effects
            .iter()
            .find_map(|e| match e {
                Effect::SavePortfolioRecord { record, .. } => Some(record.clone()),
                _ => None,
            })
            .unwrap();
        let (state, followup) = acknowledge(state, &effects);
        if day == now() {
            assert!(effects.iter().any(|e| matches!(e, Effect::LogMorningArchiveExcusals { articles, blocked: false, .. } if articles[0].url == "https://meter.example/retry" && articles[0].stages == [FailedStage::Scoring])));
            assert!(followup.iter().any(|e| matches!(e, Effect::ArchiveRequested { ordered_urls, .. } if ordered_urls == &["https://meter.example/other"])));
        } else {
            assert_outcome(
                &state,
                BatchArchiveStatus::CycleFailed,
                Some(BatchArchiveReason::Backlog),
            );
        }
    }
}

fn apply_portfolio_writes(stored: &mut BriefingCheckpoint, effects: &[Effect]) {
    for effect in effects {
        if let Effect::SavePortfolioRecord {
            target_key,
            record,
            checkpoint,
            ..
        } = effect
        {
            merge_portfolio_record(stored, target_key, record, *checkpoint);
        }
    }
}

#[test]
fn window_sequence_accounts_for_membership_export_uniqueness_and_selection() {
    let bootstrap = "2026-09-27T06:00:00.123456Z";
    let boundary = "2026-09-30T10:00:00Z";
    let skipped = "2026-09-28T08:00:00Z";
    let backlog_day = "2026-09-29T08:30:00Z";
    let second = "2026-10-01T11:00:00Z";
    let quiet_day = "2026-10-02T11:15:00Z";
    let final_until = "2026-10-03T12:00:00Z";
    let future = "2026-10-03T11:30:00Z";
    let rows = [
        ("first", bootstrap, 99, "first"),
        ("boundary", boundary, 99, "boundary"),
        ("future", future, 99, "future"),
        ("folded", bootstrap, 95, "first"),
        ("excluded", bootstrap, 90, "excluded"),
        ("low", bootstrap, 1, "low"),
    ];
    let mut fixture = Fixture::new(&rows.map(|(name, _, _, _)| (name, Some(5), true, None)));
    let configured = replies(AppState::new());
    for (a, (_, fetched, value, cluster)) in fixture.articles.iter_mut().zip(rows) {
        a.fetched_utc = Some(fetched.into());
        let skey = configured
            .current_summary_cache_key(&a.content_hash)
            .unwrap();
        let key = crate::update::signal_candidate::input_key_for_current_results(
            &configured,
            a,
            &triage(5),
            &skey,
            &summary(),
        )
        .unwrap();
        fixture.scores.insert(
            key,
            SignalCandidateCacheEntry {
                result: score(cluster, value),
                created_at_utc: DATE.into(),
            },
        );
    }
    fixture.exclusions.insert(exclusion("excluded"));
    let mut backlog_fixture = fixture.clone();
    let mut incomplete = article("incomplete");
    incomplete.fetched_utc = Some(bootstrap.into());
    backlog_fixture.articles.push(incomplete);
    let mut cp = BriefingCheckpoint {
        since_utc: Some(DESKTOP.into()),
        ..Default::default()
    };
    let mut confirmed = Vec::new();
    let mut previous = None;
    let mut blocked_window = None;
    for (until, sidecar, publish, confirm) in [
        (skipped, sidecar(bootstrap), true, false),
        (backlog_day, SidecarObservation::Parsed(None), false, false),
        (boundary, SidecarObservation::Parsed(None), true, true),
        (second, self::sidecar(boundary), true, true),
        (quiet_day, self::sidecar(second), false, false),
        (final_until, self::sidecar(second), true, true),
    ] {
        let seen = observation(sidecar, previous);
        let resolved = resolve_window(&cp, &key(A), &seen, now()).unwrap();
        let active = if until == backlog_day {
            &backlog_fixture
        } else {
            &fixture
        };
        let (state, startup_effects) =
            set_up(metadata_replies(active.hydrate()), cp.clone(), seen, false);
        let state = ready_after_setup(
            state,
            &startup_effects,
            &active.articles,
            &resolved.since_utc.to_rfc3339(),
            |effects| apply_portfolio_writes(&mut cp, effects),
        );
        let threshold = state.signal_candidate_threshold();
        let excluded = state.signal_exclusions().excluded();
        let deliberate_omissions: HashSet<_> = state
            .saved_results_entries()
            .filter(|entry| {
                entry.signal.as_ref().is_some_and(|score| {
                    score.signal_score < threshold
                        || excluded.contains(&exclusion(&score.signal_key))
                })
            })
            .map(|entry| entry.article.url.clone())
            .collect();
        let (state, effects) = request(state, date(until));
        apply_portfolio_writes(&mut cp, &effects);
        if !publish {
            assert!(effects.is_empty());
            if until == backlog_day {
                assert_outcome(
                    &state,
                    BatchArchiveStatus::CycleFailed,
                    Some(BatchArchiveReason::Backlog),
                );
                blocked_window = Some((
                    ArchiveWindow::from_submission(Some(resolved.since_utc), date(until)),
                    state.batch_archive_outcome().unwrap().backlog_articles,
                ));
            } else {
                assert_outcome(&state, BatchArchiveStatus::NothingNew, None);
            }
            continue;
        }
        assert_eq!(cp.portfolio[&key(A)].pending_until_utc, Some(date(until)));
        let (state, effects) = acknowledge(state, &effects);
        let (window, ordered_urls, folds) = effects
            .into_iter()
            .find_map(|effect| match effect {
                Effect::ArchiveRequested {
                    window,
                    ordered_urls,
                    folds,
                    ..
                } => Some((window, ordered_urls, folds)),
                _ => None,
            })
            .expect("durable intent releases publication");
        previous = Some(windowed(&resolved.since_utc.to_rfc3339(), until));
        let (state, save) = completed(state);
        if until == boundary {
            let (state, _) = update(
                state,
                Msg::PortfolioRecordSaved {
                    save_id: save_id(&save),
                    result: Err("crash before checkpoint".into()),
                },
            );
            assert_outcome(
                &state,
                BatchArchiveStatus::Exported,
                Some(BatchArchiveReason::CheckpointNotSaved),
            );
            let stored = &cp.portfolio[&key(A)];
            assert_eq!(stored.pending_until_utc, Some(date(boundary)));
            assert_eq!(stored.last_archive_until_utc, Some(date(skipped)));
        } else {
            apply_portfolio_writes(&mut cp, &save);
            let _ = acknowledge(state, &save);
        }
        if confirm {
            confirmed.push((window, ordered_urls, folds, deliberate_omissions));
        }
        if until == boundary {
            // Set the desktop checkpoint, run B against its own seed, then clear
            // the desktop checkpoint before returning to A and reconciling P.
            cp.set_since_utc(Some(boundary.into()));
            let (other, startup_effects) = set_up_for(
                metadata_replies(fixture.hydrate()),
                B,
                cp.clone(),
                observation(SidecarObservation::Missing, None),
                true,
            );
            let other = ready_after_setup(
                other,
                &startup_effects,
                &fixture.articles,
                boundary,
                |effects| apply_portfolio_writes(&mut cp, effects),
            );
            let (other, save) = request(other, date(second));
            apply_portfolio_writes(&mut cp, &save);
            let (other, effects) = acknowledge(other, &save);
            assert!(effects
                .iter()
                .any(|effect| matches!(effect, Effect::ArchiveRequested { .. })));
            let (other, save) = completed(other);
            apply_portfolio_writes(&mut cp, &save);
            let _ = acknowledge(other, &save);
            assert_eq!(
                cp.portfolio[&key(B)].confirmed_until_utc,
                Some(date(boundary))
            );
            cp.set_since_utc(None);
        }
    }
    // 1. Window membership is exhaustive and disjoint for dated corpus articles.
    for article in &fixture.articles {
        assert_eq!(
            confirmed
                .iter()
                .filter(|(w, _, _, _)| w.contains(article.fetched_utc.as_deref()))
                .count(),
            1,
            "{}",
            article.url
        );
    }
    // 2. Each exported URL belongs to that archive's half-open window.
    for (window, urls, _, _) in &confirmed {
        assert!(urls.iter().all(|url| fixture
            .articles
            .iter()
            .any(|a| a.url == *url && window.contains(a.fetched_utc.as_deref()))));
    }
    // 3. No document appears in two confirmed archives.
    let mut once = HashSet::new();
    for (_, urls, _, _) in &confirmed {
        for url in urls {
            assert!(once.insert(url));
        }
    }
    // 4. Each unexported member has a concrete selection or backlog explanation.
    let (blocked_window, backlog) = blocked_window.expect("backlog blocked one window");
    assert!(blocked_window.contains(Some(bootstrap)) && backlog == 1);
    for (window, urls, folds, deliberate_omissions) in &confirmed {
        for a in fixture
            .articles
            .iter()
            .filter(|a| window.contains(a.fetched_utc.as_deref()) && !urls.contains(&a.url))
        {
            assert!(
                deliberate_omissions.contains(&a.url)
                    || folds.values().flatten().any(|member| member.url == a.url),
                "unexplained omission: {}",
                a.url
            );
        }
    }
}
