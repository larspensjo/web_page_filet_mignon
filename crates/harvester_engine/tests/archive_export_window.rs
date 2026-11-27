use harvester_engine::{build_triage_archive_with, ArchiveTarget, ArchiveWindow, ExportSummary};
use std::{fs, path::Path};

fn export(
    output: &Path,
    target: ArchiveTarget<'_>,
    urls: &[String],
    since: Option<&str>,
) -> ExportSummary {
    build_triage_archive_with(
        output,
        target,
        urls,
        ArchiveWindow::from_submission(
            since.map(|date| date.parse().unwrap()),
            "2026-11-27T07:00:00.9Z".parse().unwrap(),
        ),
        false,
        &Default::default(),
        &Default::default(),
        &Default::default(),
        None,
        None,
        &mut |_| {},
    )
    .unwrap()
}

#[test]
fn archive_external_target_preserves_the_manifest_location_and_desktop_basename_rule() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    let path = root.path().join("archive.md");
    fs::create_dir(&output).unwrap();
    let manifest = output.join("harvester-corpus.json");
    fs::write(&manifest, b"obsolete manifest").unwrap();
    fs::write(output.join("archive-renamed.md"), b"archive artifact").unwrap();
    let summary = export(&output, ArchiveTarget::External { path: &path }, &[], None);
    assert_eq!(summary.output_path, path);
    assert_eq!(summary.files_read, 0);
    assert!(path.is_file());
    assert_ne!(fs::read(manifest).unwrap(), b"obsolete manifest");
    assert!(!output.join("archive.md").exists());
    assert!(!root.path().join("harvester-corpus.json").exists());

    for basename in ["../escape.md", "..\\escape.md", path.to_str().unwrap()] {
        let result = build_triage_archive_with(
            &output,
            ArchiveTarget::InOutput { basename },
            &[],
            ArchiveWindow::from_submission(None, "2026-11-27T07:00:01Z".parse().unwrap()),
            false,
            &Default::default(),
            &Default::default(),
            &Default::default(),
            None,
            None,
            &mut |_| {},
        );
        assert!(result.is_err(), "accepted {basename}");
    }
    assert_eq!(
        export(
            &output,
            ArchiveTarget::InOutput {
                basename: "custom.md"
            },
            &[],
            None
        )
        .output_path,
        output.join("custom.md")
    );
}

#[test]
fn archive_filters_a_fractional_half_open_window_and_writes_schema_two_fields() {
    let root = tempfile::tempdir().unwrap();
    let mut urls = Vec::new();
    for (name, date) in [
        ("old", "2026-11-27T07:00:00.123455Z"),
        ("since", "2026-11-27T08:00:00.123456+01:00"),
        ("until", "2026-11-27T07:00:01Z"),
        ("future", "2029-01-01T00:00:00Z"),
        ("malformed", "unknown"),
    ] {
        let url = format!("https://example.com/{name}");
        fs::write(
            root.path().join(format!("{name}.md")),
            format!(
                "---\nurl: \"{url}\"\ntitle: \"{name}\"\nfetched_utc: \"{date}\"\n---\n\nbody\n"
            ),
        )
        .unwrap();
        urls.push(url);
    }
    let summary = export(
        root.path(),
        ArchiveTarget::InOutput {
            basename: "archive.md",
        },
        &urls,
        Some("2026-11-27T07:00:00.123456Z"),
    );
    assert_eq!(summary.doc_count, 2);
    assert_eq!(summary.window_count, Some(2));
    assert_eq!(summary.unexported_by_priority, Some([0; 6]));
    let archive = fs::read_to_string(summary.output_path).unwrap();
    for name in ["since", "malformed"] {
        assert!(archive.contains(&format!("url: https://example.com/{name}\n")));
    }
    for name in ["old", "until", "future"] {
        assert!(!archive.contains(&format!("url: https://example.com/{name}\n")));
    }
    assert!(archive.contains("\nexport_schema: 2\n"));
    assert!(archive.contains("\nwindow_since_utc: 2026-11-27T07:00:00.123456Z\nwindow_until_utc: 2026-11-27T07:00:01Z\ndoc | line |"));

    let target = ArchiveTarget::InOutput {
        basename: "archive.md",
    };
    let summary = export(root.path(), target, &urls, Some("2026-11-27T07:00:00.000Z"));
    let archive = fs::read_to_string(summary.output_path).unwrap();
    assert!(archive.contains("\nwindow_since_utc: 2026-11-27T07:00:00Z\n"));
    let summary = export(root.path(), target, &urls, None);
    assert_eq!(summary.doc_count, 3);
    assert_eq!(summary.window_count, None);
    let archive = fs::read_to_string(summary.output_path).unwrap();
    assert!(!archive.contains("\nwindow_since_utc: "));
    assert!(archive.contains("\nwindow_until_utc: 2026-11-27T07:00:01Z\ndoc | line |"));
}
