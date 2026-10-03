//! Admission, downstream release and progress share the same reducer-owned ledger.
use crate::briefing::ArticleSummaryState as S;
use crate::pipeline_waves::Identity;
use crate::run_progress::StageCounts;
use crate::triage::ArticleTriageState as T;
use crate::{AppState, LoadedArticle, PipelineStage as Stage, SignalCandidateState as C};
use std::collections::HashSet;

fn run_id(state: &AppState) -> u64 {
    state.run_progress().map_or(0, |r| r.run_id)
}

pub(super) fn release(state: &mut AppState, stage: Stage, members: Vec<Identity>) {
    let cap = state.llm_max_in_flight().max(1).saturating_mul(4);
    let id = run_id(state);
    for members in members.chunks(cap) {
        state.pipeline_waves.push(id, stage, members.to_vec());
    }
}

fn admit_once(state: &mut AppState, stage: Stage, member: &Identity) -> bool {
    state
        .pipeline_admission
        .as_mut()
        .is_some_and(|r| r.admitted[stage.index() - 3].insert(member.clone()))
}

pub(super) fn admit_triage(state: &mut AppState, articles: Vec<LoadedArticle>) {
    if !state.pipeline_ready() {
        return;
    }
    record_reprocess_notice(state);
    let mut members = Vec::new();
    for article in articles {
        let member = (article.url.clone(), article.content_hash.clone());
        if !admit_once(state, Stage::Triaging, &member) {
            continue;
        }
        let key = state.current_triage_cache_key(&article.content_hash);
        let budget = state
            .pre_triage()
            .preparation_budget(&article.url)
            .or(state.processing_budget);
        state.triage_mut().admit(article, key, budget);
        members.push(member);
    }
    // A new run independently admits summaries whose triage is already current,
    // even if a previous run recorded the upstream release marker.
    let ready = summary_articles(state, &members)
        .into_iter()
        .filter(|a| {
            state
                .current_triage_cache_key(&a.content_hash)
                .is_some_and(|key| {
                    state
                        .pipeline_waves
                        .summary_released
                        .contains(&((a.url.clone(), a.content_hash.clone()), key))
                })
        })
        .collect();
    release(state, Stage::Triaging, members.clone());
    let mut reused = 0;
    for member in &members {
        let index = state
            .triage()
            .index_for_identity(&member.0, &member.1)
            .expect("admitted triage");
        let current = matches!(
            state.triage().articles()[index].triage_state,
            T::Completed { .. }
        ) && state
            .current_triage_cache_key(&member.1)
            .is_some_and(|key| {
                state.triage().articles()[index].cache_key_snapshot.as_ref() == Some(&key)
            });
        let pending = matches!(state.triage().articles()[index].triage_state, T::Pending);
        if current {
            triage_changed(state, &member.0, &member.1);
            super::signal_candidate::try_enqueue(state, &member.0);
        }
        if current
            || (pending
                && matches!(
                    super::reuse::reuse_triage(state, index),
                    super::reuse::TriageReuseOutcome::Hit
                ))
        {
            record_reused(state, Stage::Triaging, member);
            reused += 1;
        }
    }
    log_reused(state, Stage::Triaging, members.len(), reused);
    admit_summaries(state, ready);
    state.mark_dirty();
}

fn record_reprocess_notice(state: &mut AppState) {
    let Some(run) = state.pipeline_admission.as_ref() else {
        return;
    };
    if run.initial_admitted {
        return;
    }
    let previous = run.previous_window.clone();
    let (articles, calls) = state.reprocess_counts(&previous);
    let remaining = state
        .llm_quota()
        .limits
        .as_ref()
        .and_then(|l| l.max_calls_per_session)
        .map(|limit| limit.saturating_sub(state.llm_quota().usage.calls));
    let notice = crate::evaluate_reprocess_notice(articles, calls, remaining);
    engine_logging::engine_info!(
        "[pipeline-reprocess] run_id={} articles={} estimated_calls={} notice={}",
        run_id(state),
        articles,
        calls,
        notice
    );
    let run = state.pipeline_admission.as_mut().unwrap();
    run.initial_admitted = true;
    if notice {
        run.reprocess_notice = Some((articles, calls));
    }
}

fn summary_articles(state: &AppState, members: &[Identity]) -> Vec<LoadedArticle> {
    members.iter().filter_map(|(url, hash)| state.triage().index_for_identity(url, hash).map(|i| &state.triage().articles()[i])).filter(|a| {
        let Some(key) = state.current_triage_cache_key(&a.content_hash) else { return false; };
        matches!(&a.triage_state, T::Completed { result } if a.cache_key_snapshot.as_ref() == Some(&key) && result.priority > state.briefing_triage_policy().cutoff_exclusive)
    }).map(|a| LoadedArticle { url: a.url.clone(), source_title: a.source_title.clone(), prepared_text: a.prepared_text.clone(), content_hash: a.content_hash.clone(), fetched_utc: a.fetched_utc.clone() }).collect()
}

pub(super) fn admit_summaries(state: &mut AppState, articles: Vec<LoadedArticle>) {
    admit_summary_wave(state, articles);
}

fn admit_summary_wave(state: &mut AppState, articles: Vec<LoadedArticle>) {
    if !state.pipeline_ready() {
        return;
    }
    let mut members = Vec::new();
    for article in articles {
        let member = (article.url.clone(), article.content_hash.clone());
        if !admit_once(state, Stage::Summarizing, &member) {
            continue;
        }
        let key = state.current_summary_cache_key(&article.content_hash).ok();
        state.briefing_mut().admit(article, key);
        members.push(member);
    }
    if !members.is_empty() {
        state.mark_briefing_metadata_ready();
        release(state, Stage::Summarizing, members.clone());
    }
    let mut reused = 0;
    for member in &members {
        let index = state
            .briefing()
            .index_for_identity(&member.0, &member.1)
            .expect("admitted summary");
        let current = matches!(
            state.briefing().articles()[index].summary_state,
            S::Completed { .. }
        ) && state
            .current_summary_cache_key(&member.1)
            .ok()
            .is_some_and(|key| state.briefing().article_cache_key(index) == Some(&key));
        let pending = matches!(state.briefing().articles()[index].summary_state, S::Pending);
        if current || (pending && super::reuse::reuse_summary(state, index)) {
            record_reused(state, Stage::Summarizing, member);
            reused += 1;
        }
        super::signal_candidate::try_enqueue(state, &member.0);
    }
    log_reused(state, Stage::Summarizing, members.len(), reused);
}

pub(super) fn release_ready(state: &mut AppState) {
    if !state.pipeline_ready() {
        return;
    }
    let changed = std::mem::take(&mut state.pipeline_waves.changed_triage_waves);
    for index in changed {
        let wave = &state.pipeline_waves.waves[index];
        if wave.members.iter().any(|(url, hash)| {
            state
                .triage()
                .index_for_identity(url, hash)
                .is_some_and(|i| {
                    matches!(
                        state.triage().articles()[i].triage_state,
                        T::Pending | T::InProgress { .. }
                    )
                })
        }) {
            continue;
        }
        let members = wave.members.clone();
        let articles = summary_articles(state, &members)
            .into_iter()
            .filter(|a| {
                state
                    .current_triage_cache_key(&a.content_hash)
                    .is_some_and(|k| {
                        !state
                            .pipeline_waves
                            .summary_released
                            .contains(&((a.url.clone(), a.content_hash.clone()), k))
                    })
            })
            .collect::<Vec<_>>();
        for a in &articles {
            if let Some(key) = state.current_triage_cache_key(&a.content_hash) {
                state
                    .pipeline_waves
                    .summary_released
                    .insert(((a.url.clone(), a.content_hash.clone()), key));
            }
        }
        admit_summary_wave(state, articles);
    }
}

pub(super) fn scoring_admitted(state: &mut AppState, member: Identity, digest: String) -> bool {
    if !state.pipeline_ready() {
        return false;
    }
    if !admit_once(state, Stage::ScoringSignals, &member) {
        return false;
    }
    state
        .pipeline_waves
        .scoring_released
        .insert((member.clone(), digest));
    release(state, Stage::ScoringSignals, vec![member]);
    true
}

pub(super) fn prune(state: &mut AppState, members: HashSet<Identity>) {
    let old: std::collections::HashMap<_, _> = state
        .triage()
        .articles()
        .iter()
        .map(|a| (a.url.clone(), a.content_hash.clone()))
        .collect();
    let urls = members
        .iter()
        .filter(|m| !old.get(&m.0).is_some_and(|hash| hash != &m.1))
        .map(|m| m.0.clone())
        .collect();
    state.triage_mut().retain_members(&members);
    state.briefing_mut().retain_members(&members);
    state.signal_candidate_mut().retain_urls(&urls);
    state.pipeline_waves.retain(&members);
}

pub(super) fn triage_changed(state: &mut AppState, url: &str, hash: &str) {
    if let Some(index) = state
        .pipeline_waves
        .triage_wave
        .get(&(url.to_owned(), hash.to_owned()))
        .copied()
    {
        state.pipeline_waves.changed_triage_waves.insert(index);
    }
}

pub(super) fn next_article(state: &mut AppState, stage: Stage) -> Option<usize> {
    let queue = stage.index() - 3;
    while let Some((url, hash)) = state.pipeline_waves.pending[queue].front() {
        let index = if stage == Stage::Triaging {
            state.triage().index_for_identity(url, hash)
        } else {
            state.briefing().index_for_identity(url, hash)
        };
        if let Some(index) = index {
            let pending = if stage == Stage::Triaging {
                matches!(state.triage().articles()[index].triage_state, T::Pending)
            } else {
                matches!(state.briefing().articles()[index].summary_state, S::Pending)
            };
            if pending {
                return Some(index);
            }
        }
        state.pipeline_waves.pending[queue].pop_front();
    }
    if stage == Stage::Triaging {
        state.triage().next_pending_index()
    } else {
        state.briefing().next_pending_index()
    }
}

pub(super) fn next_score(state: &mut AppState) -> Option<String> {
    while let Some((url, _)) = state.pipeline_waves.pending[2].front() {
        if matches!(state.signal_candidate().state_for(url), Some(C::Pending)) {
            return Some(url.clone());
        }
        state.pipeline_waves.pending[2].pop_front();
    }
    state
        .signal_candidate()
        .next_pending_url()
        .map(str::to_owned)
}

pub(super) fn record_progress(state: &mut AppState) {
    let Some(run) = state.pipeline_admission.as_ref() else {
        return;
    };
    let observed: [Vec<_>; 3] = std::array::from_fn(|stage| {
        run.admitted[stage]
            .iter()
            .filter_map(|member| {
                if run.completed[stage].contains(member) || run.failed[stage].contains(member) {
                    return None;
                }
                let settlement =
                    match stage {
                        0 => state
                            .triage()
                            .index_for_identity(&member.0, &member.1)
                            .map(|i| match state.triage().articles()[i].triage_state {
                                T::Completed { .. } => (true, false),
                                T::Failed { .. } => (false, true),
                                _ => (false, false),
                            }),
                        1 => state
                            .briefing()
                            .index_for_identity(&member.0, &member.1)
                            .map(|i| match state.briefing().articles()[i].summary_state {
                                S::Completed { .. } => (true, false),
                                S::Failed { .. } => (false, true),
                                _ => (false, false),
                            }),
                        _ => state
                            .signal_candidate()
                            .state_for(&member.0)
                            .map(|s| match s {
                                C::Completed { .. } => (true, false),
                                C::Failed { .. } => (false, true),
                                _ => (false, false),
                            }),
                    }?;
                (settlement.0 || settlement.1).then(|| (member.clone(), settlement))
            })
            .collect()
    });
    let run = state.pipeline_admission.as_mut().expect("run admission");
    let counts: [_; 3] = std::array::from_fn(|stage| {
        for (member, (completed, failed)) in &observed[stage] {
            if *completed {
                run.completed[stage].insert(member.clone());
            }
            if *failed {
                run.failed[stage].insert(member.clone());
            }
        }
        // Reuse checks each insertion itself. Recheck the full relationships
        // when observed settlements change them, rather than on idle passes.
        if !observed[stage].is_empty() {
            debug_assert!(run.reused[stage].is_subset(&run.completed[stage]));
            debug_assert!(run.completed[stage].is_disjoint(&run.failed[stage]));
            debug_assert!(run.completed[stage].is_subset(&run.admitted[stage]));
            debug_assert!(run.failed[stage].is_subset(&run.admitted[stage]));
        }
        StageCounts {
            completed: run.completed[stage].len() as u32,
            failed: run.failed[stage].len() as u32,
            total: run.admitted[stage].len() as u32,
            reused: run.reused[stage].len() as u32,
        }
    });
    let intake_final = !run.intake_open;
    let triage_final =
        intake_final && state.triage().pending_count() + state.triage().in_progress_count() == 0;
    let summary_final = triage_final
        && state.briefing().pending_count() + state.briefing().in_progress_count() == 0;
    let now = state.last_observed_utc();
    let stopping = state.pipeline_run_phase() == crate::PipelineRunPhase::Stopping;
    let Some(progress) = state.run_progress_mut() else {
        return;
    };
    for (stage, counts, total_is_final) in [
        (Stage::Triaging, counts[0], intake_final),
        (Stage::Summarizing, counts[1], triage_final),
        (Stage::ScoringSignals, counts[2], summary_final),
    ]
    .into_iter()
    {
        if counts.total > 0 {
            progress.counts(stage, counts, now);
        } else if !total_is_final && !stopping {
            progress.activate(stage, 0, now);
        }
        progress.stage_mut(stage).total_is_final = total_is_final || stopping;
    }
    progress.counts(
        Stage::LoadingArticles,
        StageCounts {
            completed: counts[0].total,
            total: counts[0].total,
            ..StageCounts::default()
        },
        now,
    );
    if intake_final {
        progress.finish(Stage::LoadingArticles, now);
    }
    progress.stage_mut(Stage::LoadingArticles).total_is_final = intake_final || stopping;
}

pub(super) fn record_reused(state: &mut AppState, stage: Stage, member: &Identity) {
    let run = state.pipeline_admission.as_mut().expect("run admission");
    let index = stage.index() - 3;
    debug_assert!(run.admitted[index].contains(member));
    debug_assert!(!run.failed[index].contains(member));
    run.reused[index].insert(member.clone());
    run.completed[index].insert(member.clone());
    debug_assert!(run.completed[index].contains(member));
}

pub(super) fn log_reused(state: &AppState, stage: Stage, admitted: usize, reused: usize) {
    if reused > 0 {
        engine_logging::engine_info!(
            "[pipeline-reuse] run_id={} stage={:?} admitted={} reused={}",
            run_id(state),
            stage,
            admitted,
            reused
        );
    }
}
