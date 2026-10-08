use harvester_core::{update, AppState, Effect, JobResultKind, Msg, Stage};
use harvester_engine::{ExtractedLink, LinkKind};

fn submit_urls(state: AppState, input: &str) -> (AppState, Vec<Effect>) {
    let (state, _) = update(state, Msg::InputChanged(input.to_string()));
    update(state, Msg::UrlsSubmitted)
}

#[test]
fn urls_pasted_trims_and_ignores_empty() {
    let state = AppState::new();
    let input = "https://a.example.com \n\n  https://b.example.com\n   \n";

    let (mut next, _effects) = submit_urls(state, input);
    let view = next.view();

    assert_eq!(view.job_count, 2);
    assert!(next.consume_dirty());
}

#[test]
fn job_progress_updates_stage_tokens_and_bytes() {
    let state = AppState::new();
    let (state, _) = submit_urls(state, "https://a.example.com\nhttps://b.example.com\n");

    let (mut next, _) = update(
        state,
        Msg::JobProgress {
            job_id: 1,
            stage: Stage::Downloading,
            tokens: Some(10),
            bytes: Some(1024),
        },
    );
    let job1 = next
        .view()
        .desktop_job_list
        .rows
        .iter()
        .find(|j| j.job_id == 1)
        .unwrap()
        .clone();
    assert_eq!(job1.stage, Stage::Downloading);
    assert_eq!(job1.tokens, Some(10));
    assert_eq!(job1.bytes, Some(1024));
    assert!(next.consume_dirty());
}

#[test]
fn job_done_transitions_to_done() {
    let state = AppState::new();
    let (state, _) = submit_urls(state, "https://a.example.com\nhttps://b.example.com\n");

    let (mut next, _) = update(
        state,
        Msg::JobDone {
            job_id: 1,
            result: JobResultKind::Success,
            extracted_links: Vec::new(),
            fetched_utc: None,
        },
    );
    let job1 = next
        .view()
        .desktop_job_list
        .rows
        .iter()
        .find(|j| j.job_id == 1)
        .unwrap()
        .clone();
    assert_eq!(job1.stage, Stage::Done);
    assert_eq!(job1.outcome, Some(JobResultKind::Success));
    assert!(next.consume_dirty());
}

#[test]
fn jobs_are_ordered_by_btree_key() {
    let state = AppState::new();
    let (mut state, _effects) = submit_urls(state, "b.com\na.com\n");

    // BTreeMap iteration should yield deterministic ascending JobId order (1,2,...)
    let ids: Vec<_> = state
        .view()
        .desktop_job_list
        .rows
        .iter()
        .map(|j| j.job_id)
        .collect();
    assert_eq!(ids, vec![1, 2]);
    assert!(state.consume_dirty());
}

#[test]
fn job_done_attaches_link_records_and_dedupes() {
    let state = AppState::new();
    let (state, _effects) = submit_urls(state, "https://links.example\n");
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id: 1,
            result: JobResultKind::Success,
            extracted_links: vec![
                ExtractedLink {
                    url: "HTTP://EXAMPLE.com".to_string(),
                    text: Some("Primary".to_string()),
                    kind: LinkKind::Hyperlink,
                },
                ExtractedLink {
                    url: "http://example.com/".to_string(),
                    text: Some("Duplicate".to_string()),
                    kind: LinkKind::Hyperlink,
                },
                ExtractedLink {
                    url: "https://example.com/image.png".to_string(),
                    text: Some("Image".to_string()),
                    kind: LinkKind::Image,
                },
                ExtractedLink {
                    url: "https://example.com/guide".to_string(),
                    text: None,
                    kind: LinkKind::Hyperlink,
                },
            ],
            fetched_utc: None,
        },
    );
    let links = state.job_links(1).expect("job links available");
    assert_eq!(links.len(), 3);
    assert_eq!(links[0].index, 0);
    assert_eq!(links[0].url, "http://example.com/".to_string());
    assert_eq!(links[0].anchor_text.as_deref(), Some("Primary"));
    assert_eq!(links[0].kind, LinkKind::Hyperlink);

    assert_eq!(links[1].index, 2);
    assert_eq!(links[1].kind, LinkKind::Image);
    assert_eq!(links[1].anchor_text.as_deref(), Some("Image"));

    assert_eq!(links[2].index, 3);
    assert_eq!(links[2].url, "https://example.com/guide".to_string());
    assert!(links[2].anchor_text.is_none());
}
