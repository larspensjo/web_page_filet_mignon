use crate::archive_publication::*;
use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

const NEW: &[u8] = b"complete new archive";
const OLD: &[u8] = b"window_until_utc: 2030-01-01T00:00:00Z\n===== ARCHIVE INDEX =====\nexport_schema: 2\nfetched_to: 2026-11-25T01:02:03Z\nwindow_until_utc: 2026-11-26T02:03:04.123456+01:00\ndoc | line | fetched_utc | priority | signal_key | title\n===== INDEX END =====\n";
const KEPT: &str = "archive-unprocessed-20261126T010304Z.md";
const STALE_AGE: Duration = Duration::from_secs(3600);

fn file_names(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn archive_publication_replaces_observed_target_and_can_publish_to_an_absent_target() {
    use harvester_engine::{prepare_triage_archive_with, ArchiveTarget, ArchiveWindow};

    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("archive.md");
    assert_eq!(
        publish_archive(&target, OLD, PreviousArchiveDisposition::None)
            .unwrap()
            .previous_kept,
        None
    );
    let observed = ArchiveTargetObservation::read(&target).unwrap();
    let output = directory.path().join("output");
    let prepared = prepare_triage_archive_with(
        &output,
        ArchiveTarget::External { path: &target },
        &[],
        ArchiveWindow::from_submission(None, "2026-11-27T07:00:00Z".parse().unwrap()),
        false,
        &Default::default(),
        &Default::default(),
        &Default::default(),
        None,
        None,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(fs::read(&target).unwrap(), OLD);
    let (summary, published) = prepared
        .publish(|path, contents| {
            publish_archive(
                path,
                contents.as_bytes(),
                PreviousArchiveDisposition::Replace { observed },
            )
        })
        .unwrap();
    assert_eq!(published.previous_kept, None);
    assert_eq!(summary.output_path, target);
    assert!(fs::read_to_string(&target)
        .unwrap()
        .contains("\nwindow_until_utc: 2026-11-27T07:00:00Z\n"));
    assert_eq!(file_names(directory.path()), ["archive.md", "output"]);
    assert!(output.join("harvester-corpus.json").is_file());
}

#[cfg(windows)]
#[test]
fn archive_publication_replaces_target_open_for_read_and_clears_temporary_attribute() {
    use std::io::Read;
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x100;
    for keep_aside in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("archive.md");
        fs::write(&target, OLD).unwrap();
        let mut reader = File::open(&target).unwrap();
        let disposition = if keep_aside {
            PreviousArchiveDisposition::KeepAside
        } else {
            PreviousArchiveDisposition::Replace {
                observed: ArchiveTargetObservation::read(&target).unwrap(),
            }
        };

        let result = publish_archive(&target, NEW, disposition).unwrap();
        assert_eq!(fs::read(&target).unwrap(), NEW);
        assert_eq!(
            fs::metadata(&target).unwrap().file_attributes() & FILE_ATTRIBUTE_TEMPORARY,
            0
        );
        assert_eq!(
            result.previous_kept,
            keep_aside.then(|| directory.path().join(KEPT))
        );
        if let Some(path) = result.previous_kept {
            assert_eq!(fs::read(path).unwrap(), OLD);
        }
        let mut old = Vec::new();
        reader.read_to_end(&mut old).unwrap();
        assert_eq!(old, OLD);
    }
}

#[test]
fn archive_publication_keeps_old_bytes_with_stamp_from_index_then_mtime() {
    let modification: chrono::DateTime<chrono::Utc> = "2026-11-24T01:02:03Z".parse().unwrap();
    let legacy = b"===== ARCHIVE INDEX =====\nexport_schema: 2\nfetched_to: 2026-11-25T01:02:03Z\ndoc | line | fetched_utc | priority | signal_key | title\n===== INDEX END =====\n";
    for (old, name) in [
        (OLD, KEPT),
        (legacy.as_slice(), "archive-unprocessed-20261125T010203Z.md"),
        (
            b"window_until_utc: 2030-01-01T00:00:00Z\n".as_slice(),
            "archive-unprocessed-20261124T010203Z.md",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("archive.md");
        fs::write(&target, old).unwrap();
        File::options()
            .write(true)
            .open(&target)
            .unwrap()
            .set_modified(modification.into())
            .unwrap();
        let published =
            publish_archive(&target, NEW, PreviousArchiveDisposition::KeepAside).unwrap();
        let path = directory.path().join(name);
        assert_eq!(published.previous_kept.as_deref(), Some(path.as_path()));
        assert_eq!(fs::read(path).unwrap(), old);
        assert_eq!(fs::read(&target).unwrap(), NEW);
        assert_eq!(file_names(directory.path()).len(), 2);
    }
}

#[test]
fn archive_publication_uses_the_first_free_suffix_and_reuses_an_identical_copy() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("archive.md");
    let first = directory.path().join(KEPT);
    let second = directory
        .path()
        .join("archive-unprocessed-20261126T010304Z-2.md");
    let third = directory
        .path()
        .join("archive-unprocessed-20261126T010304Z-3.md");
    fs::write(first, b"first clash").unwrap();
    for expected in [&second, &third, &third] {
        fs::write(&target, OLD).unwrap();
        let published =
            publish_archive(&target, NEW, PreviousArchiveDisposition::KeepAside).unwrap();
        assert_eq!(published.previous_kept.as_ref(), Some(expected));
        assert_eq!(fs::read(expected).unwrap(), OLD);
        assert_eq!(fs::read(&target).unwrap(), NEW);
        if expected == &second {
            // Keep the original name occupied by a different complete archive.
            fs::write(&second, b"second clash").unwrap();
        }
    }
    assert_eq!(file_names(directory.path()).len(), 4);
}

#[cfg(windows)]
#[test]
fn archive_publication_skips_unreadable_kept_copy_and_keeps_the_old_target() {
    use std::os::windows::fs::OpenOptionsExt;

    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("archive.md");
    let unreadable = directory.path().join(KEPT);
    let next_kept = directory
        .path()
        .join("archive-unprocessed-20261126T010304Z-2.md");
    fs::write(&target, OLD).unwrap();
    fs::write(&unreadable, OLD).unwrap();
    let lock = File::options()
        .write(true)
        .share_mode(0)
        .open(&unreadable)
        .unwrap();
    assert!(File::open(&unreadable).is_err());

    let published = publish_archive(&target, NEW, PreviousArchiveDisposition::KeepAside).unwrap();
    assert_eq!(published.previous_kept.as_ref(), Some(&next_kept));
    assert_eq!(fs::read(next_kept).unwrap(), OLD);
    assert_eq!(fs::read(target).unwrap(), NEW);
    drop(lock);
    assert_eq!(fs::read(unreadable).unwrap(), OLD);
}

#[cfg(windows)]
#[test]
fn archive_publication_continues_after_a_stale_temp_cannot_be_deleted() {
    use std::os::windows::fs::OpenOptionsExt;

    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("archive.md");
    let blocked = directory.path().join(".archive.md.tmp-blocked");
    let removable = directory.path().join(".archive.md.tmp-removable");
    let now = SystemTime::now();
    for path in [&blocked, &removable] {
        fs::write(path, b"stale staging").unwrap();
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(now - STALE_AGE - Duration::from_secs(1))
            .unwrap();
    }
    let lock = File::options()
        .read(true)
        .share_mode(1 | 2) // FILE_SHARE_READ | FILE_SHARE_WRITE
        .open(&blocked)
        .unwrap();
    assert!(fs::remove_file(&blocked).is_err());

    publish_archive_with(&target, NEW, PreviousArchiveDisposition::None, now, |_| {
        Ok(())
    })
    .unwrap();
    assert_eq!(fs::read(target).unwrap(), NEW);
    assert!(blocked.exists());
    assert!(!removable.exists());
    drop(lock);
}

#[test]
fn archive_publication_keeps_a_target_changed_since_observation() {
    for mtime_only in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("archive.md");
        fs::write(&target, OLD).unwrap();
        let observed = ArchiveTargetObservation::read(&target).unwrap();
        let changed = if mtime_only {
            OLD.to_vec()
        } else {
            [b"changed\n", OLD].concat()
        };
        fs::write(&target, &changed).unwrap();
        let modified = if mtime_only {
            SystemTime::now() - STALE_AGE
        } else {
            observed.modified
        };
        File::options()
            .write(true)
            .open(&target)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        let published = publish_archive(
            &target,
            NEW,
            PreviousArchiveDisposition::Replace { observed },
        )
        .unwrap();
        assert_eq!(
            published.previous_kept.as_ref(),
            Some(&directory.path().join(KEPT))
        );
        assert_eq!(fs::read(published.previous_kept.unwrap()).unwrap(), changed);
        assert_eq!(fs::read(target).unwrap(), NEW);
    }
}

#[test]
fn archive_publication_failure_during_keep_or_replace_preserves_old_target() {
    for (failure, kept) in [
        (PublishStep::SyncedKeptArchive, false),
        (PublishStep::KeptPrevious, true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("archive.md");
        let kept_path = directory.path().join(KEPT);
        fs::write(&target, OLD).unwrap();
        let error = publish_archive_with(
            &target,
            NEW,
            PreviousArchiveDisposition::KeepAside,
            SystemTime::now(),
            |step| {
                if step == failure {
                    return Err(io::Error::other("injected publication failure"));
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(fs::read(&target).unwrap(), OLD);
        assert_eq!(
            error.previous_kept.as_deref(),
            kept.then_some(kept_path.as_path())
        );
        assert_eq!(file_names(directory.path()).len(), if kept { 2 } else { 1 });
        if kept {
            assert_eq!(fs::read(kept_path).unwrap(), OLD);
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("archive.md");
        let kept_path = directory.path().join(KEPT);
        fs::write(&target, OLD).unwrap();
        let mut reader = None;
        let error = publish_archive_with(
            &target,
            NEW,
            PreviousArchiveDisposition::KeepAside,
            SystemTime::now(),
            |step| {
                if step == PublishStep::KeptPrevious {
                    // Windows readers can allow reads and writes while denying rename/delete.
                    reader = Some(File::options().read(true).share_mode(1 | 2).open(&target)?);
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert!(reader.is_some());
        assert_eq!(fs::read(&target).unwrap(), OLD);
        assert_eq!(error.previous_kept.as_ref(), Some(&kept_path));
        assert_eq!(fs::read(kept_path).unwrap(), OLD);
        assert_eq!(file_names(directory.path()).len(), 2);
    }
}

#[test]
fn archive_publication_crash_boundaries_have_only_complete_visible_files_and_age_out_temps() {
    const CRASH: &str = "HARVESTER_PUBLICATION_TEST_CRASH";
    const TARGET: &str = "HARVESTER_PUBLICATION_TEST_TARGET";
    let steps = [
        PublishStep::CleanedStaleTemps,
        PublishStep::SyncedNewArchive,
        PublishStep::CheckedTarget,
        PublishStep::SyncedKeptArchive,
        PublishStep::KeptPrevious,
        PublishStep::ReplacedTarget,
        PublishStep::ReportedCompletion,
    ];
    if let Some(raw) = std::env::var_os(CRASH) {
        let index = raw.to_str().unwrap().parse::<usize>().unwrap();
        let target = std::path::PathBuf::from(std::env::var_os(TARGET).unwrap());
        publish_archive_with(
            &target,
            NEW,
            PreviousArchiveDisposition::KeepAside,
            SystemTime::now(),
            |step| {
                if step == steps[index] {
                    std::process::exit(71);
                }
                Ok(())
            },
        )
        .unwrap();
        panic!("fault point not reached");
    }
    for (index, step) in steps.into_iter().enumerate() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("archive.md");
        let kept = directory.path().join(KEPT);
        fs::write(&target, OLD).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["archive_publication_crash_boundaries_have_only_complete_visible_files_and_age_out_temps", "--quiet"])
            .env(CRASH, index.to_string()).env(TARGET, &target).output().unwrap().status;
        assert_eq!(status.code(), Some(71), "{step:?}");
        assert_eq!(
            fs::read(&target).unwrap(),
            if index < 5 { OLD } else { NEW },
            "{step:?}"
        );
        if index < 4 {
            assert!(!kept.exists(), "{step:?}");
        } else {
            assert_eq!(fs::read(kept).unwrap(), OLD, "{step:?}");
        }
        let temps: Vec<_> = file_names(directory.path())
            .into_iter()
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert_eq!(
            temps.len(),
            match index {
                1 | 2 | 4 => 1,
                3 => 2,
                _ => 0,
            },
            "{step:?}"
        );
        for name in &temps {
            assert!(
                name.starts_with(".archive.md.tmp-") || name.starts_with(&format!(".{KEPT}.tmp-")),
                "{name}"
            );
        }
        let fail_after_cleanup = |stage| {
            assert_eq!(stage, PublishStep::CleanedStaleTemps);
            Err(io::Error::other("stop after stale-temp cleanup"))
        };
        let latest = temps
            .iter()
            .map(|name| {
                fs::metadata(directory.path().join(name))
                    .unwrap()
                    .modified()
                    .unwrap()
            })
            .max()
            .unwrap_or_else(SystemTime::now);
        let earliest = temps
            .iter()
            .map(|name| {
                fs::metadata(directory.path().join(name))
                    .unwrap()
                    .modified()
                    .unwrap()
            })
            .min()
            .unwrap_or(latest);
        let unrelated = directory.path().join(".other.md.tmp-unrelated");
        fs::write(&unrelated, b"unrelated").unwrap();
        File::options()
            .write(true)
            .open(&unrelated)
            .unwrap()
            .set_modified(earliest - STALE_AGE)
            .unwrap();
        for (now, still_exists) in [
            (earliest + STALE_AGE, true),
            (latest + STALE_AGE + Duration::from_secs(1), false),
        ] {
            publish_archive_with(
                &target,
                NEW,
                PreviousArchiveDisposition::KeepAside,
                now,
                fail_after_cleanup,
            )
            .unwrap_err();
            for name in &temps {
                assert_eq!(directory.path().join(name).exists(), still_exists, "{name}");
            }
            assert!(unrelated.exists());
        }
    }
}
