use super::*;
use crate::morning_archive::*;
use chrono::{DateTime, Utc};
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

fn load_window(state: AppState, articles: &[LoadedArticle], since: &str) -> AppState {
    load_window_with_projection(state, articles, articles, since)
}

fn load_window_with_projection(
    mut state: AppState,
    articles: &[LoadedArticle],
    saved: &[LoadedArticle],
    since: &str,
) -> AppState {
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
                return resolve(scan(state, request_id, saved), request_id, articles);
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
fn morning_reports_missing_results_even_when_the_pipeline_has_no_work() {
    let mut fixture = Fixture::new(&[
        ("low", Some(1), false, None),
        ("excluded", None, false, None),
        ("missing-a", None, false, None),
        ("missing-b", None, false, None),
        ("missing-c", None, false, None),
        ("needs-summary", Some(5), false, None),
    ]);
    fixture.articles[1].prepared_text = "tiny".into();
    fixture.articles[4].source_title = None;
    let (state, effects) = set_up(
        metadata_replies(fixture.hydrate()),
        persisted(A, record(START, Some(LAST), None)),
        observation(SidecarObservation::Missing, None),
        true,
    );
    assert!(effects.is_empty());
    let state =
        load_window_with_projection(state, &fixture.articles[..2], &fixture.articles, START);
    assert_eq!(state.startup_readiness(), Readiness::Ready);
    for article in &fixture.articles[2..5] {
        assert!(state.pre_triage().entry_for_url(&article.url).is_none());
    }
    assert!(matches!(
        state.unfinished_work(),
        crate::UnfinishedWork::Known(work) if work.articles_with_work == 0
    ));
    let (state, effects) = request(state, now());
    let (state, effects) = acknowledge(state, &effects);
    assert!(effects.is_empty());
    assert_outcome(&state, BatchArchiveStatus::NothingNew, None);
    assert_eq!(
        state.batch_archive_outcome().unwrap().left_out,
        fixture.articles[2..]
            .iter()
            .map(|article| LeftOutArticle {
                url: article.url.clone(),
                title: article.source_title.clone(),
                reason: "not_finished".into(),
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn morning_triages_redirects_and_keeps_legacy_matches_before_the_window_out() {
    let redirect =
        "https://digitimes.com/news/a20261126VL214/huawei-ascend-chips-nvidia-atlas.html";
    let final_url =
        "https://www.digitimes.com/newsshow/article.asp?datePublish=2026/11/26&pages=vl&seq=214";
    let old_requested = "https://example.com/old-request";
    let old_url = "https://example.com/old-article";
    let old_date = "2026-10-02T09:00:00Z";
    let dir = tempfile::tempdir().unwrap();
    for (filename, url, fetched, word) in [
        ("today.md", final_url, DATE, "in-window "),
        ("old.md", old_url, old_date, "historical "),
    ] {
        std::fs::write(
            dir.path().join(filename),
            format!(
                "---\nurl: {url}\ntitle: {filename}\nfetched_utc: {fetched}\n---\n\n{}",
                word.repeat(300)
            ),
        )
        .unwrap();
    }
    let state = Fixture::new(&[]).hydrate();
    let state = update(
        state,
        Msg::RestoreCompletedJobs(vec![CompletedJobSnapshot {
            url: old_requested.into(),
            tokens: Some(1000),
            bytes: None,
            links: vec![],
            final_url: None,
            fetched_utc: Some(old_date.into()),
        }]),
    )
    .0;
    let state = update(state, Msg::InputChanged(redirect.into())).0;
    let (state, effects) = update(state, Msg::UrlsSubmitted);
    let job_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::EnqueueUrl { job_id, .. } => Some(*job_id),
            _ => None,
        })
        .expect("requested URL is enqueued");
    let state = update(
        state,
        Msg::JobDone {
            job_id,
            result: crate::JobResultKind::Success,
            extracted_links: vec![],
            final_url: Some(final_url.into()),
            fetched_utc: Some(DATE.into()),
        },
    )
    .0;
    let (state, effects) = set_up(
        metadata_replies(state),
        persisted(A, record(START, Some(LAST), None)),
        observation(SidecarObservation::Missing, None),
        true,
    );
    assert!(effects.is_empty());
    let (state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    let registry = harvester_engine::llm::PromptRegistry::with_defaults();
    let input_limit = 20_000;
    let budget = harvester_engine::summary_preparation_budget(input_limit, &registry).unwrap();
    let (state, effects) =
        crate::fixture_support::complete_processing_configuration(state, effects, budget);
    let Effect::LoadArticlesForTriage {
        request_id,
        ordered_entries,
        since_utc,
        held,
    } = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::LoadArticlesForTriage { .. }))
        .expect("morning pre-triage load")
    else {
        unreachable!()
    };
    assert_eq!(since_utc, Some(date(START)));
    assert!(ordered_entries.iter().any(|entry| matches!(
        entry,
        harvester_engine::TriageLoadEntry::Job { id: Some(id), final_url: Some(url), .. }
            if *id == job_id && url == final_url
    )));
    let mut index = harvester_engine::CorpusScanIndex::default();
    let (whole_corpus, _) = index
        .load_delta(
            dir.path(),
            input_limit,
            &registry,
            &ordered_entries,
            None,
            &held,
            Some(request_id),
            |_| {},
        )
        .unwrap();
    assert!(whole_corpus
        .members
        .iter()
        .any(|article| article.url == old_url));
    let (delta, _) = index
        .load_delta(
            dir.path(),
            input_limit,
            &registry,
            &ordered_entries,
            since_utc,
            &held,
            Some(request_id),
            |_| {},
        )
        .unwrap();
    let state = crate::update(
        state,
        Msg::SavedArticlesLoaded {
            request_id,
            articles: index.article_metadata(),
        },
    )
    .0;
    let (mut state, mut effects) =
        crate::update(state, Msg::TriageArticlesLoaded { request_id, delta });
    assert_eq!(
        state
            .triage()
            .articles()
            .iter()
            .map(|article| article.url.as_str())
            .collect::<Vec<_>>(),
        [final_url]
    );

    for (prompt, model, output_json) in [
        (
            PromptId::ArticleTriage,
            "triage-model",
            r#"{"category":"news","priority":5,"tags":["ai"],"rationale":"ok"}"#,
        ),
        (
            PromptId::ArticleSummary,
            "summary-model",
            r#"{"title":"Summary","summary":"Summary","key_points":["point"]}"#,
        ),
        (
            PromptId::ArticleSignalCandidate,
            "score-model",
            r#"{"signal_score":95,"signal_key":"redirect","themes":["ai"],"draft_gist":"Publisher reports an infrastructure event.","source_tier":"Tier1","confidence":"High","reasoning":"Concrete event."}"#,
        ),
    ] {
        let request_id = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::RequestLlmCompletion {
                    request_id,
                    prompt_id,
                    ..
                } if *prompt_id == prompt => Some(*request_id),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{prompt:?} was not dispatched"));
        (state, effects) = crate::update(
            state,
            Msg::LlmCompleted {
                request_id,
                result: crate::LlmResultKind::Success {
                    output_json: output_json.into(),
                    input_tokens: 10,
                    output_tokens: 5,
                    prompt_version: 1,
                    resolved_model: model.into(),
                },
                metadata: None,
            },
        );
    }
    let (state, effect) = export(state, now());
    assert!(matches!(
        effect,
        Effect::ArchiveRequested { window, ordered_urls, .. }
            if window.since_utc() == Some(date(START)) && window.until_utc() == now()
                && ordered_urls == [final_url]
    ));
    let (state, effects) = completed(state);
    let (state, _) = acknowledge(state, &effects);
    assert_outcome(&state, BatchArchiveStatus::Exported, None);
    assert!(state.batch_archive_outcome().unwrap().left_out.is_empty());
}

#[test]
fn scoring_failure_is_left_out_and_other_articles_are_published() {
    let fixture = Fixture::new(&[
        ("retry", Some(5), true, None),
        ("other", Some(5), true, Some((95, "other"))),
    ]);
    for failure in [
        crate::LlmResultKind::Failed {
            reason: "fixture failure".into(),
        },
        crate::LlmResultKind::RateLimited {
            reason: "isolated throttle".into(),
        },
    ] {
        let state = fail_article(
            normal(&fixture),
            &fixture.articles[0],
            FailedStage::Scoring,
            failure,
        );
        let (state, effects) = request(state, now());
        let (state, publication) = acknowledge(state, &effects);
        assert!(publication.iter().any(|effect| matches!(
            effect,
            Effect::ArchiveRequested { ordered_urls, .. } if ordered_urls == &["https://meter.example/other"]
        )));
        let (state, save) = completed(state);
        let (state, _) = acknowledge(state, &save);
        assert_outcome(&state, BatchArchiveStatus::Exported, None);
        assert_eq!(
            state.batch_archive_outcome().unwrap().left_out,
            [LeftOutArticle {
                url: fixture.articles[0].url.clone(),
                title: fixture.articles[0].source_title.clone(),
                reason: "failed:scoring".into(),
            }]
        );
    }
}

#[test]
fn quota_halt_blocks_even_with_a_selection_and_keeps_the_window() {
    use crate::state::ModelDispatchHalt;
    let mut fixture = Fixture::selected();
    fixture.articles.push(article("unfinished"));
    for halt in [
        ModelDispatchHalt::RunQuota("limit".into()),
        ModelDispatchHalt::ProviderCredits("credits".into()),
        ModelDispatchHalt::RateLimit("rate".into()),
    ] {
        let mut state = normal(&fixture);
        state.halt_model_dispatch(halt);
        let (state, effects) = request(state, now());
        assert!(effects.is_empty());
        assert_outcome(
            &state,
            BatchArchiveStatus::CycleFailed,
            Some(BatchArchiveReason::QuotaHalted),
        );
        let outcome = state.batch_archive_outcome().unwrap();
        assert!(outcome.left_out.is_empty());
        assert_eq!(outcome.window, None);
        assert_eq!(state.batch_archive_since_utc(), Some(date(START)));
    }
}

#[test]
fn stop_before_archive_blocks_without_saving_omissions() {
    let (state, effects) = update(
        normal(&Fixture::new(&[("withdrawn", Some(5), false, None)])),
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    assert!(effects.contains(&Effect::PollAllSources));
    let state = update(state, Msg::StopFinishClicked).0;
    assert_cycle_failed(state, BatchArchiveReason::Interrupted);
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
    let mut fixture = Fixture::selected();
    fixture.articles.push(article("unfinished"));
    let (state, effects) = request(normal(&fixture), now());
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
    assert!(state.batch_archive_outcome().unwrap().left_out.is_empty());
}

#[test]
fn failed_nothing_new_record_write_does_not_report_omissions() {
    let fixture = Fixture::new(&[("unfinished", None, false, None)]);
    let (state, effects) = request(normal(&fixture), now());
    let (state, effects) = update(
        state,
        Msg::PortfolioRecordSaved {
            save_id: save_id(&effects),
            result: Err("record refused".into()),
        },
    );
    assert!(effects.is_empty());
    assert_outcome(
        &state,
        BatchArchiveStatus::CycleFailed,
        Some(BatchArchiveReason::RecordNotSaved),
    );
    assert!(state.batch_archive_outcome().unwrap().left_out.is_empty());
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
    let mut fixture = Fixture::selected();
    let unfinished = article("unfinished");
    fixture.articles.push(unfinished.clone());
    let mut checkpoint = persisted(A, record(START, Some(LAST), None));
    let (state, save) = request(normal(&fixture), now());
    apply_portfolio_writes(&mut checkpoint, &save);
    assert!(checkpoint.portfolio[&key(A)]
        .pending_left_out
        .contains_key(&unfinished.url));
    let (state, effects) = acknowledge(state, &save);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::ArchiveRequested { .. })));
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
    assert!(state.batch_archive_outcome().unwrap().left_out.is_empty());

    let retry = ready(
        &fixture,
        checkpoint,
        observation(SidecarObservation::Missing, None),
        true,
        START,
    );
    let (retry, _) = export(retry, now());
    let (retry, save) = completed(retry);
    let (retry, _) = acknowledge(retry, &save);
    assert_outcome(&retry, BatchArchiveStatus::Exported, None);
    assert_eq!(
        retry.batch_archive_outcome().unwrap().left_out[0].url,
        unfinished.url
    );
}

fn fail_article(
    mut state: AppState,
    retry: &LoadedArticle,
    stage: FailedStage,
    result: crate::LlmResultKind,
) -> AppState {
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
    update(
        state,
        Msg::LlmCompleted {
            request_id,
            result,
            metadata: None,
        },
    )
    .0
}

#[test]
fn a_left_out_article_reports_each_failed_stage_once_in_pipeline_order() {
    let fixture = Fixture::new(&[("retry", Some(5), false, None)]);
    let article = &fixture.articles[0];
    let mut state = fail_article(
        normal(&fixture),
        article,
        FailedStage::Summary,
        crate::LlmResultKind::Failed {
            reason: "fixture failure".into(),
        },
    );
    let morning = state.morning_archive.as_mut().unwrap();
    for stage in [
        FailedStage::Scoring,
        FailedStage::Summary,
        FailedStage::Triage,
        FailedStage::Summary,
    ] {
        morning.note_failure(&article.url, stage);
    }
    let (state, save) = request(state, now());
    let (state, _) = acknowledge(state, &save);
    assert_outcome(&state, BatchArchiveStatus::NothingNew, None);
    assert_eq!(
        state.batch_archive_outcome().unwrap().left_out,
        [LeftOutArticle {
            url: article.url.clone(),
            title: article.source_title.clone(),
            reason: "failed:triage+summary+scoring".into(),
        }]
    );
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
fn relaxed_fetch_timestamp_is_retained_as_undated_after_confirmation() {
    let relaxed = "2026-10-03T09:00:00UTC";
    assert!(date(relaxed) < date(LAST));
    assert!(DateTime::parse_from_rfc3339(relaxed).is_err());

    let mut fixture = Fixture::selected();
    let mut unfinished = article("unfinished");
    unfinished.fetched_utc = Some(relaxed.into());
    let url = unfinished.url.clone();
    fixture.articles.push(unfinished);

    let mut checkpoint = persisted(A, record(START, Some(LAST), None));
    let (_, intent) = request(normal(&fixture), date(LAST));
    apply_portfolio_writes(&mut checkpoint, &intent);
    assert!(checkpoint.portfolio[&key(A)]
        .pending_left_out
        .contains_key(&url));

    let confirmed = resolve_window(
        &checkpoint,
        &key(A),
        &observation(sidecar(LAST), Some(windowed(START, LAST))),
        now(),
    )
    .unwrap();
    assert_eq!(confirmed.since_utc, date(LAST));
    assert_eq!(confirmed.record.left_out.get(&url), Some(&None));
}

#[test]
fn window_sequence_accounts_for_membership_export_uniqueness_and_selection() {
    let bootstrap = "2026-09-27T06:00:00.123456Z";
    let boundary = "2026-09-30T10:00:00Z";
    let skipped = "2026-09-28T08:00:00Z";
    let quota_day = "2026-09-29T08:30:00Z";
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
    let mut incomplete = article("incomplete");
    incomplete.fetched_utc = Some(bootstrap.into());
    let mut incomplete_next = article("incomplete-next");
    incomplete_next.fetched_utc = Some(boundary.into());
    let mut incomplete_replay = article("incomplete-replay");
    incomplete_replay.fetched_utc = Some("2026-09-29T09:00:00Z".into());
    let mut incomplete_quiet = article("incomplete-quiet");
    incomplete_quiet.fetched_utc = Some("2026-10-02T09:00:00Z".into());
    let omissions = [
        incomplete,
        incomplete_replay,
        incomplete_next,
        incomplete_quiet,
    ];
    fixture.articles.extend(omissions.iter().cloned());
    let mut recovered = fixture.clone();
    let formerly_incomplete = &omissions[0];
    let triage_key = configured
        .current_triage_cache_key(&formerly_incomplete.content_hash)
        .unwrap();
    let summary_key = configured
        .current_summary_cache_key(&formerly_incomplete.content_hash)
        .unwrap();
    let signal_key = crate::update::signal_candidate::input_key_for_current_results(
        &configured,
        formerly_incomplete,
        &triage(5),
        &summary_key,
        &summary(),
    )
    .unwrap();
    recovered.triages.insert_entry(
        triage_key,
        TriageCacheEntry {
            result: triage(5),
            created_at_utc: DATE.into(),
        },
    );
    recovered.summaries.insert(
        summary_key,
        SummaryCacheEntry {
            result: summary(),
            created_at_utc: DATE.into(),
        },
    );
    recovered.scores.insert(
        signal_key,
        SignalCandidateCacheEntry {
            result: score("recovered", 99),
            created_at_utc: DATE.into(),
        },
    );
    let mut cp = BriefingCheckpoint {
        since_utc: Some(DESKTOP.into()),
        ..Default::default()
    };
    let mut confirmed = Vec::new();
    let mut previous = None;
    let mut reported = HashSet::new();
    for (until, sidecar, publish, confirm) in [
        (skipped, sidecar(bootstrap), true, false),
        (quota_day, SidecarObservation::Parsed(None), false, false),
        (boundary, SidecarObservation::Parsed(None), true, true),
        (second, self::sidecar(boundary), true, true),
        (quiet_day, self::sidecar(second), false, false),
        (final_until, self::sidecar(second), true, true),
    ] {
        let seen = observation(sidecar, previous);
        let resolved = resolve_window(&cp, &key(A), &seen, now()).unwrap();
        let active = if until == boundary {
            &recovered
        } else {
            &fixture
        };
        let (state, startup_effects) =
            set_up(metadata_replies(active.hydrate()), cp.clone(), seen, false);
        let mut state = ready_after_setup(
            state,
            &startup_effects,
            &fixture.articles,
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
        if until == quota_day {
            state.halt_model_dispatch(crate::state::ModelDispatchHalt::RunQuota("limit".into()));
        }
        let (state, effects) = request(state, date(until));
        apply_portfolio_writes(&mut cp, &effects);
        if !publish {
            if until == quota_day {
                assert!(effects.is_empty());
                assert_outcome(
                    &state,
                    BatchArchiveStatus::CycleFailed,
                    Some(BatchArchiveReason::QuotaHalted),
                );
                assert_eq!(state.batch_archive_since_utc(), Some(date(bootstrap)));
            } else {
                let (state, effects) = acknowledge(state, &effects);
                assert!(effects.is_empty());
                assert_outcome(&state, BatchArchiveStatus::NothingNew, None);
                let left_out = &state.batch_archive_outcome().unwrap().left_out;
                assert_eq!(left_out.len(), 1);
                assert_eq!(left_out[0].url, omissions[3].url);
                assert!(reported.insert(left_out[0].url.clone()));
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
        let state = if until == boundary {
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
            assert!(stored.pending_left_out.contains_key(&omissions[1].url));
            assert!(!ordered_urls.contains(&formerly_incomplete.url));
            state
        } else {
            apply_portfolio_writes(&mut cp, &save);
            let (state, _) = acknowledge(state, &save);
            assert_outcome(&state, BatchArchiveStatus::Exported, None);
            state
        };
        for omission in &state.batch_archive_outcome().unwrap().left_out {
            assert_eq!(omission.reason, "not_finished");
            assert!(fixture.articles.iter().any(|a| a.url == omission.url
                && a.source_title == omission.title
                && window.contains(a.fetched_utc.as_deref())));
            assert!(reported.insert(omission.url.clone()));
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
    // 4. Each unexported member has a concrete selection or left-out explanation.
    assert_eq!(reported, omissions.into_iter().map(|a| a.url).collect());
    for (window, urls, folds, deliberate_omissions) in &confirmed {
        for a in fixture
            .articles
            .iter()
            .filter(|a| window.contains(a.fetched_utc.as_deref()) && !urls.contains(&a.url))
        {
            assert!(
                deliberate_omissions.contains(&a.url)
                    || reported.contains(&a.url)
                    || folds.values().flatten().any(|member| member.url == a.url),
                "unexplained omission: {}",
                a.url
            );
        }
    }
}
