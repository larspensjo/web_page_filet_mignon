use super::*;

#[test]
fn signal_candidate_rows_leave_gists_empty_for_scoring_and_failed_states() {
    let mut state = AppState::new();
    let scoring_url = "https://example.com/signal/scoring/".to_string() + &"a".repeat(96);
    let failed_url = "https://example.com/signal/failed/".to_string() + &"b".repeat(96);

    insert_done_job(&mut state, 1, &scoring_url);
    insert_done_job(&mut state, 2, &failed_url);
    state.signal_candidate_mut().enqueue(scoring_url.clone());
    state.signal_candidate_mut().mark_scoring(&scoring_url, 7);
    state.signal_candidate_mut().enqueue(failed_url.clone());
    state.signal_candidate_mut().fail(&failed_url, "boom");

    let rows = state.build_signal_candidate_rows();

    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.gist_truncated.is_empty()));
}

fn complete_candidate(
    state: &mut AppState,
    url: &str,
    score: u8,
    signal_key: &str,
    tier: harvester_engine::llm::dto::SourceTier,
    gist: &str,
) {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult};
    state.signal_candidate_mut().enqueue(url.to_string());
    state.signal_candidate_mut().mark_scoring(url, 1);
    state.signal_candidate_mut().complete(
        url,
        SignalCandidateResult {
            signal_score: score,
            signal_key: signal_key.to_string(),
            themes: vec!["theme".to_string()],
            draft_gist: gist.to_string(),
            source_tier: tier,
            confidence: Confidence::High,
            reasoning: "r".to_string(),
            input_tokens: 100,
            output_tokens: 10,
        },
    );
}

fn outcome_for<'a>(
    rows: &'a [SignalCandidateRow],
    url: &str,
) -> &'a Option<SignalCandidateOutcome> {
    &rows
        .iter()
        .find(|r| r.url == url)
        .expect("row present")
        .outcome
}

#[test]
fn outcome_classifies_selected_dedup_and_below_threshold() {
    use harvester_engine::llm::dto::SourceTier;
    let mut state = AppState::new();
    state.set_signal_candidate_threshold(60);

    let rep = "https://example.com/rep/".to_string() + &"a".repeat(96);
    let dupe = "https://example.com/dupe/".to_string() + &"b".repeat(96);
    let low = "https://example.com/low/".to_string() + &"c".repeat(96);
    insert_done_job(&mut state, 1, &rep);
    insert_done_job(&mut state, 2, &dupe);
    insert_done_job(&mut state, 3, &low);

    // rep and dupe share a signal_key; rep is Tier1 so it wins the cluster.
    complete_candidate(
        &mut state,
        &rep,
        90,
        "shared",
        SourceTier::Tier1,
        "kept gist text",
    );
    complete_candidate(
        &mut state,
        &dupe,
        85,
        "shared",
        SourceTier::Tier2,
        "dupe gist text",
    );
    complete_candidate(
        &mut state,
        &low,
        50,
        "solo",
        SourceTier::Tier1,
        "low gist text",
    );

    let rows = state.build_signal_candidate_rows();

    assert_eq!(
        outcome_for(&rows, &rep),
        &Some(SignalCandidateOutcome::Selected)
    );
    assert_eq!(
        outcome_for(&rows, &dupe),
        &Some(SignalCandidateOutcome::Deduplicated {
            kept_gist: "kept gist text".to_string()
        })
    );
    assert_eq!(
        outcome_for(&rows, &low),
        &Some(SignalCandidateOutcome::BelowThreshold)
    );
}

#[test]
fn outcome_deduplicates_canonical_model_access_restriction_keys() {
    use harvester_engine::llm::dto::SourceTier;
    let mut state = AppState::new();
    state.set_signal_candidate_threshold(60);

    let rep = "https://example.com/anthropic-rep/".to_string() + &"a".repeat(96);
    let dupe = "https://example.com/anthropic-dupe/".to_string() + &"b".repeat(96);
    insert_done_job(&mut state, 1, &rep);
    insert_done_job(&mut state, 2, &dupe);
    complete_candidate(
        &mut state,
        &rep,
        92,
        "anthropic-model-access-suspension-foreign-national-order",
        SourceTier::Tier2,
        "kept Anthropic access restriction gist",
    );
    complete_candidate(
        &mut state,
        &dupe,
        86,
        "anthropic-disables-fable-mythos-export-controls",
        SourceTier::Tier2,
        "duplicate Anthropic access restriction gist",
    );

    let rows = state.build_signal_candidate_rows();
    let dupe_row = rows.iter().find(|row| row.url == dupe).expect("dupe row");

    assert_eq!(
        outcome_for(&rows, &rep),
        &Some(SignalCandidateOutcome::Selected)
    );
    assert_eq!(
        &dupe_row.outcome,
        &Some(SignalCandidateOutcome::Deduplicated {
            kept_gist: "kept Anthropic access restriction gist".to_string()
        })
    );
    assert_eq!(dupe_row.dupes_count, 1);
}

#[test]
fn outcome_marks_excluded_clusters() {
    use harvester_engine::llm::dto::SourceTier;
    let mut state = AppState::new();
    state.set_signal_candidate_threshold(60);
    let url = "https://example.com/excl/".to_string() + &"d".repeat(96);
    insert_done_job(&mut state, 1, &url);
    complete_candidate(&mut state, &url, 90, "drop-me", SourceTier::Tier1, "gist");

    let version = state
        .active_version_for(harvester_engine::llm::prompt::PromptId::ArticleSignalCandidate)
        .unwrap_or_default();
    state
        .signal_candidate_mut()
        .add_exclusion(crate::signal_candidate::OverrideKey {
            signal_key: "drop-me".to_string(),
            prompt_id: harvester_engine::llm::prompt::PromptId::ArticleSignalCandidate.to_string(),
            prompt_version: version,
        });

    let rows = state.build_signal_candidate_rows();
    assert_eq!(
        outcome_for(&rows, &url),
        &Some(SignalCandidateOutcome::Excluded)
    );
}

#[test]
fn rows_order_selected_then_dedup_then_below_threshold() {
    use harvester_engine::llm::dto::SourceTier;
    let mut state = AppState::new();
    state.set_signal_candidate_threshold(60);
    let rep = "https://example.com/o-rep/".to_string() + &"a".repeat(96);
    let dupe = "https://example.com/o-dupe/".to_string() + &"b".repeat(96);
    let low = "https://example.com/o-low/".to_string() + &"c".repeat(96);
    insert_done_job(&mut state, 1, &rep);
    insert_done_job(&mut state, 2, &dupe);
    insert_done_job(&mut state, 3, &low);
    complete_candidate(&mut state, &rep, 90, "shared", SourceTier::Tier1, "rep");
    complete_candidate(&mut state, &dupe, 88, "shared", SourceTier::Tier2, "dupe");
    complete_candidate(&mut state, &low, 50, "solo", SourceTier::Tier1, "low");

    let rows = state.build_signal_candidate_rows();
    let order: Vec<&Option<SignalCandidateOutcome>> = rows.iter().map(|r| &r.outcome).collect();
    assert_eq!(order[0], &Some(SignalCandidateOutcome::Selected));
    assert!(matches!(
        order[1],
        Some(SignalCandidateOutcome::Deduplicated { .. })
    ));
    assert_eq!(order[2], &Some(SignalCandidateOutcome::BelowThreshold));
}
