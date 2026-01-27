use std::path::PathBuf;

use harvester_core::{
    update, AppState, Effect, JobResultKind, LinkDownloadState, Msg, Stage, TOKEN_LIMIT,
};
use harvester_engine::{ExtractedLink, LinkKind};

fn submit_urls(state: AppState, input: &str) -> (AppState, Vec<Effect>) {
    let (state, _) = update(state, Msg::InputChanged(input.to_string()));
    update(state, Msg::UrlsSubmitted)
}

fn state_with_single_link() -> AppState {
    let (state, _) = submit_urls(AppState::new(), "https://links.example\n");
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id: 1,
            result: JobResultKind::Success,
            content_preview: None,
            extracted_links: vec![ExtractedLink {
                url: "http://example.com/".to_string(),
                text: Some("Example".to_string()),
                kind: LinkKind::Hyperlink,
            }],
        },
    );
    state
}

fn init_logging() {
    engine_logging::initialize_for_tests();
}

#[test]
fn urls_pasted_trims_and_ignores_empty() {
    let state = AppState::new();
    let input = "https://a.example.com \n\n  https://b.example.com\n   \n";

    let (mut next, _effects) = submit_urls(state, input);
    let view = next.view();

    assert!(view.queued_urls.is_empty());
    assert_eq!(view.job_count, 2);
    assert!(next.consume_dirty());

    // Progress on job 1.
    let (mut next, _effects) = update(
        next,
        Msg::JobProgress {
            job_id: 1,
            stage: Stage::Downloading,
            tokens: Some(10),
            bytes: Some(1024),
            content_preview: None,
        },
    );
    let job1 = next
        .view()
        .jobs
        .iter()
        .find(|j| j.job_id == 1)
        .unwrap()
        .clone();
    assert_eq!(job1.stage, Stage::Downloading);
    assert_eq!(job1.tokens, Some(10));
    assert_eq!(job1.bytes, Some(1024));
    assert!(next.consume_dirty());

    // Completion for job 1.
    let (mut next, _effects) = update(
        next,
        Msg::JobDone {
            job_id: 1,
            result: JobResultKind::Success,
            content_preview: None,
            extracted_links: Vec::new(),
        },
    );
    let job1_done = next
        .view()
        .jobs
        .iter()
        .find(|j| j.job_id == 1)
        .unwrap()
        .clone();
    assert_eq!(job1_done.stage, Stage::Done);
    assert_eq!(job1_done.outcome, Some(JobResultKind::Success));
    assert!(next.consume_dirty());
}

#[test]
fn jobs_are_ordered_by_btree_key() {
    let state = AppState::new();
    let (mut state, _effects) = submit_urls(state, "b.com\na.com\n");

    // BTreeMap iteration should yield deterministic ascending JobId order (1,2,...)
    let ids: Vec<_> = state.view().jobs.iter().map(|j| j.job_id).collect();
    assert_eq!(ids, vec![1, 2]);
    assert!(state.consume_dirty());
}

#[test]
fn token_totals_accumulate_and_replace_previous_values() {
    let state = AppState::new();
    let (state, _effects) = submit_urls(state, "a.com\nb.com\n");

    let (mut state, _effects) = update(
        state,
        Msg::JobProgress {
            job_id: 1,
            stage: Stage::Tokenizing,
            tokens: Some(120),
            bytes: None,
            content_preview: None,
        },
    );
    let view_after_first = state.view();
    assert_eq!(view_after_first.total_tokens, 120);
    assert_eq!(view_after_first.token_limit, TOKEN_LIMIT);
    assert!(state.consume_dirty());

    let (mut state, _effects) = update(
        state,
        Msg::JobProgress {
            job_id: 1,
            stage: Stage::Tokenizing,
            tokens: Some(150),
            bytes: None,
            content_preview: None,
        },
    );
    assert_eq!(state.view().total_tokens, 150);
    assert!(state.consume_dirty());

    let (mut state, _effects) = update(
        state,
        Msg::JobProgress {
            job_id: 2,
            stage: Stage::Tokenizing,
            tokens: Some(50),
            bytes: None,
            content_preview: None,
        },
    );
    assert_eq!(state.view().total_tokens, 200);
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
            content_preview: None,
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

#[test]
fn link_toggle_requested_emits_download_effect() {
    init_logging();
    let state = state_with_single_link();
    let (state, effects) = update(
        state,
        Msg::LinkToggleRequested {
            job_id: 1,
            link_index: 0,
            checked: true,
        },
    );
    let link = state.job_links(1).unwrap().first().unwrap();
    assert!(matches!(
        link.download_state,
        LinkDownloadState::Downloading
    ));
    assert_eq!(
        effects,
        vec![Effect::DownloadLinkedPage {
            job_id: 1,
            link_index: 0,
            url: "http://example.com/".to_string(),
        }]
    );
}

#[test]
fn link_download_completed_updates_state() {
    init_logging();
    let path = PathBuf::from("linked/example.md");
    let (state, _) = update(
        state_with_single_link(),
        Msg::LinkDownloadCompleted {
            job_id: 1,
            link_index: 0,
            path: path.clone(),
        },
    );
    let link = state.job_links(1).unwrap().first().unwrap();
    match &link.download_state {
        LinkDownloadState::Downloaded { path: stored } => assert_eq!(stored, &path),
        other => panic!("expected downloaded state, got {:?}", other),
    }
}

#[test]
fn link_toggle_unchecked_emits_delete_effect_when_downloaded() {
    init_logging();
    let (state, _) = update(
        state_with_single_link(),
        Msg::LinkDownloadCompleted {
            job_id: 1,
            link_index: 0,
            path: PathBuf::from("linked/example.md"),
        },
    );
    let (state, effects) = update(
        state,
        Msg::LinkToggleRequested {
            job_id: 1,
            link_index: 0,
            checked: false,
        },
    );
    let link = state.job_links(1).unwrap().first().unwrap();
    assert!(matches!(
        link.download_state,
        LinkDownloadState::NotDownloaded
    ));
    assert_eq!(
        effects,
        vec![Effect::DeleteLinkedPage {
            job_id: 1,
            link_index: 0,
            path: PathBuf::from("linked/example.md"),
        }]
    );
}

#[test]
fn link_toggle_unchecked_without_download_generates_no_effect() {
    init_logging();
    let (state, effects) = update(
        state_with_single_link(),
        Msg::LinkToggleRequested {
            job_id: 1,
            link_index: 0,
            checked: false,
        },
    );
    assert!(effects.is_empty());
    let link = state.job_links(1).unwrap().first().unwrap();
    assert!(matches!(
        link.download_state,
        LinkDownloadState::NotDownloaded
    ));
}

#[test]
fn link_download_failed_sets_failed_state() {
    init_logging();
    let (state, _) = update(
        state_with_single_link(),
        Msg::LinkToggleRequested {
            job_id: 1,
            link_index: 0,
            checked: true,
        },
    );
    let (state, _) = update(
        state,
        Msg::LinkDownloadFailed {
            job_id: 1,
            link_index: 0,
            error: "boom".to_string(),
        },
    );
    let link = state.job_links(1).unwrap().first().unwrap();
    assert!(matches!(
        link.download_state,
        LinkDownloadState::Failed { ref error } if error == "boom"
    ));
}
