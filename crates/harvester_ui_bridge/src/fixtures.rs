use chrono::{DateTime, Utc};
use harvester_core::{
    update, AppState, CompletedJobSnapshot, LinkSnapshotRecord, Msg, SelectedJobVisibility,
};

use crate::{project, SnapshotEnvelope};

pub fn named_snapshots() -> Vec<(&'static str, SnapshotEnvelope)> {
    let now: DateTime<Utc> = DateTime::UNIX_EPOCH;
    let (empty, _) = update(AppState::new(), Msg::tick_at(now));
    let (with_corpus, _) = update(
        empty.clone(),
        Msg::RestoreCompletedJobs(vec![CompletedJobSnapshot {
            url: "https://example.invalid/fixture".into(),
            tokens: Some(42),
            bytes: Some(1024),
            links: vec![LinkSnapshotRecord {
                url: "https://example.invalid/link".into(),
                downloaded_path: None,
            }],
            fetched_utc: Some("1970-01-01T00:00:00Z".into()),
        }]),
    );
    let (with_selection, _) = update(
        empty.clone(),
        Msg::RestoreCompletedJobs(vec![
            CompletedJobSnapshot {
                url: "https://example.invalid/outside-checkpoint".into(),
                tokens: Some(42),
                bytes: Some(1024),
                links: vec![LinkSnapshotRecord {
                    url: "https://example.invalid/outside-checkpoint-link".into(),
                    downloaded_path: None,
                }],
                fetched_utc: Some("1970-01-01T00:00:00Z".into()),
            },
            CompletedJobSnapshot {
                url: "https://example.invalid/inside-checkpoint".into(),
                tokens: Some(43),
                bytes: Some(1025),
                links: Vec::new(),
                fetched_utc: Some("1970-01-01T00:00:02Z".into()),
            },
        ]),
    );
    let (with_selection, _) = update(
        with_selection,
        Msg::BriefingCheckpointSet(Some("1970-01-01T00:00:01Z".into())),
    );
    let (with_selection, _) = update(with_selection, Msg::JobSelected { job_id: 1 });
    let selected_view = with_selection.view();
    assert_eq!(selected_view.desktop_job_list.rows.len(), 1);
    assert!(matches!(
        selected_view
            .desktop_job_list
            .selected_job
            .as_ref()
            .map(|selected| selected.list_visibility),
        Some(SelectedJobVisibility::OutsideScope)
    ));
    vec![
        (
            "idle_empty_corpus",
            project(&empty.view()).0.with_generation(1),
        ),
        (
            "idle_with_corpus",
            project(&with_corpus.view()).0.with_generation(1),
        ),
        (
            "idle_with_selection",
            project(&selected_view).0.with_generation(1),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checked_in_snapshot_fixtures_match_core_projection() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/snapshots");
        for (name, envelope) in named_snapshots() {
            let path = root.join(format!("{name}.json"));
            let actual = serde_json::to_string_pretty(&envelope).unwrap() + "\n";
            if std::env::var_os("UPDATE_UI_FIXTURES").is_some() {
                std::fs::create_dir_all(&root).unwrap();
                std::fs::write(&path, actual).unwrap();
            } else {
                assert_eq!(
                    std::fs::read_to_string(&path).expect("checked-in UI fixture"),
                    actual,
                    "regenerate with UPDATE_UI_FIXTURES=1 cargo test -p harvester_ui_bridge"
                );
            }
        }
    }
}
