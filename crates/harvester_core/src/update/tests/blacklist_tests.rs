use super::*;
use harvester_engine::FetchOutcomeClass;

#[test]
fn fetch_outcome_classified_records_strike_for_job_domain() {
    init_logging();
    let state = AppState::default();
    // Enqueue a job with a bloomberg.com URL.
    let (state, effects) = update(
        state,
        Msg::InputChanged("https://www.bloomberg.com/news/x\n".to_string()),
    );
    let (state, effects2) = update(state, Msg::UrlsSubmitted);
    let job_id = effects
        .into_iter()
        .chain(effects2)
        .find_map(|e| match e {
            Effect::EnqueueUrl { job_id, .. } => Some(job_id),
            _ => None,
        })
        .expect("EnqueueUrl effect expected");

    let recorded_at = chrono::DateTime::from_timestamp(0, 0).unwrap();
    let mut state = state;
    for _ in 0..3 {
        let (next, effects) = update(
            state,
            Msg::FetchOutcomeClassified {
                job_id,
                class: FetchOutcomeClass::PermanentBlock,
                failure_label: Some("http status 403".to_string()),
                recorded_at,
            },
        );
        assert!(effects.is_empty(), "classification emits no effects");
        state = next;
    }
    assert!(state.blacklist().is_blocked("bloomberg.com", recorded_at));
}
