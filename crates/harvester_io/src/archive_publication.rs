//! Publish an external archive while preserving an unconfirmed previous archive.

use chrono::{DateTime, Utc};
use engine_logging::{engine_info, engine_warn};
use harvester_engine::{atomic_temp_prefix, replace_atomic_file, stage_atomic_file};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, Metadata};
use std::io::{self, BufRead, BufReader, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

const STALE_TEMP_AGE: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveTargetObservation {
    pub len: u64,
    pub modified: SystemTime,
}

impl ArchiveTargetObservation {
    pub fn read(target: &Path) -> io::Result<Self> {
        Self::from_metadata(&fs::metadata(target)?)
    }

    fn from_metadata(metadata: &Metadata) -> io::Result<Self> {
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviousArchiveDisposition {
    None,
    Replace { observed: ArchiveTargetObservation },
    KeepAside,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePublication {
    pub previous_kept: Option<PathBuf>,
    pub previous_since_utc: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
#[error("archive publication failed: {source}")]
pub struct ArchivePublicationError {
    #[source]
    pub source: io::Error,
    /// Available even when the final rename failed.
    pub previous_kept: Option<PathBuf>,
    pub previous_since_utc: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishStep {
    CleanedStaleTemps,
    SyncedNewArchive,
    CheckedTarget,
    SyncedKeptArchive,
    KeptPrevious,
    ReplacedTarget,
    ReportedCompletion,
}

/// Synchronous observation of completed publication steps.
pub type ArchivePublicationObserver = Arc<dyn Fn(PublishStep) + Send + Sync>;

pub fn publish_archive(
    target: &Path,
    archive: &[u8],
    disposition: PreviousArchiveDisposition,
) -> Result<ArchivePublication, ArchivePublicationError> {
    publish_archive_observed(target, archive, disposition, |_, _| {})
}

pub(crate) fn publish_archive_observed(
    target: &Path,
    archive: &[u8],
    disposition: PreviousArchiveDisposition,
    mut observe: impl FnMut(PublishStep, &ArchivePublication),
) -> Result<ArchivePublication, ArchivePublicationError> {
    publish_archive_impl(
        target,
        archive,
        disposition,
        SystemTime::now(),
        |step, publication| {
            observe(step, publication);
            Ok(())
        },
    )
}

#[cfg(test)]
fn publish_archive_with(
    target: &Path,
    archive: &[u8],
    disposition: PreviousArchiveDisposition,
    now: SystemTime,
    mut after_step: impl FnMut(PublishStep) -> io::Result<()>,
) -> Result<ArchivePublication, ArchivePublicationError> {
    publish_archive_impl(target, archive, disposition, now, |step, _| {
        after_step(step)
    })
}

fn publish_archive_impl(
    target: &Path,
    archive: &[u8],
    disposition: PreviousArchiveDisposition,
    now: SystemTime,
    mut after_step: impl FnMut(PublishStep, &ArchivePublication) -> io::Result<()>,
) -> Result<ArchivePublication, ArchivePublicationError> {
    engine_info!(
        "[archive-publication] starting target={} disposition={disposition:?}",
        target.display()
    );
    let mut publication = ArchivePublication {
        previous_kept: None,
        previous_since_utc: None,
    };
    let result: io::Result<()> = (|| {
        let paths = PublicationPaths::new(target)?;
        paths.remove_stale_temps(now)?;
        after_step(PublishStep::CleanedStaleTemps, &publication)?;

        let mut new = stage_atomic_file(target)?;
        new.write_all(archive)?;
        new.as_file().sync_all()?;
        after_step(PublishStep::SyncedNewArchive, &publication)?;

        let previous = match File::open(target) {
            Ok(file) => {
                let current = ArchiveTargetObservation::from_metadata(&file.metadata()?)?;
                if disposition == (PreviousArchiveDisposition::Replace { observed: current }) {
                    None
                } else {
                    if disposition != PreviousArchiveDisposition::KeepAside {
                        engine_info!("[archive-publication] retaining changed target={} disposition={disposition:?}", target.display());
                    }
                    Some((file, current))
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        after_step(PublishStep::CheckedTarget, &publication)?;

        if let Some((mut file, current)) = previous {
            publication.previous_since_utc =
                crate::archive_observation::read_archive_index(BufReader::new(&mut file))
                    .ok()
                    .and_then(|index| index.archive_start());
            file.rewind()?;
            let kept = paths.keep_previous(file, current, || {
                after_step(PublishStep::SyncedKeptArchive, &publication)
            })?;
            publication.previous_kept = Some(kept);
        }
        after_step(PublishStep::KeptPrevious, &publication)?;

        replace_atomic_file(new, target)?;
        after_step(PublishStep::ReplacedTarget, &publication)?;
        engine_info!("[archive-publication] completed target={} disposition={disposition:?} previous_kept={:?}", target.display(), publication.previous_kept);
        after_step(PublishStep::ReportedCompletion, &publication)
    })();
    match result {
        Ok(()) => Ok(publication),
        Err(source) => {
            engine_warn!("[archive-publication] failed target={} disposition={disposition:?} previous_kept={:?} reason={source}", target.display(), publication.previous_kept);
            Err(ArchivePublicationError {
                source,
                previous_kept: publication.previous_kept,
                previous_since_utc: publication.previous_since_utc,
            })
        }
    }
}

struct PublicationPaths<'a> {
    dir: &'a Path,
    new_temp_prefix: OsString,
    kept_prefix: OsString,
}

impl<'a> PublicationPaths<'a> {
    fn new(target: &'a Path) -> io::Result<Self> {
        let filename = target.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "archive target must have a filename",
            )
        })?;
        let mut kept_prefix = target
            .file_stem()
            .expect("filename has a stem")
            .to_os_string();
        kept_prefix.push("-unprocessed-");
        let dir = target
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        Ok(Self {
            dir,
            new_temp_prefix: atomic_temp_prefix(filename),
            kept_prefix,
        })
    }

    fn is_kept_name(&self, name: &[u8]) -> bool {
        let Some(suffix) = name
            .strip_prefix(self.kept_prefix.as_encoded_bytes())
            .and_then(|rest| rest.strip_suffix(b".md"))
        else {
            return false;
        };
        let Ok(suffix) = std::str::from_utf8(suffix) else {
            return false;
        };
        let (stamp, number) = suffix
            .split_once('-')
            .map_or((suffix, None), |(stamp, number)| (stamp, Some(number)));
        chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%SZ").is_ok()
            && number.is_none_or(|number| number.parse::<u64>().is_ok_and(|n| n >= 2))
    }

    fn is_temp_name(&self, name: &OsStr) -> bool {
        let bytes = name.as_encoded_bytes();
        if bytes
            .strip_prefix(self.new_temp_prefix.as_encoded_bytes())
            .is_some_and(|random| !random.is_empty())
        {
            return true;
        }
        let Some(name) = bytes.strip_prefix(b".") else {
            return false;
        };
        let Some(marker) = name
            .windows(b".tmp-".len())
            .rposition(|part| part == b".tmp-")
        else {
            return false;
        };
        name.len() > marker + b".tmp-".len() && self.is_kept_name(&name[..marker])
    }

    fn remove_stale_temps(&self, now: SystemTime) -> io::Result<()> {
        for entry in fs::read_dir(self.dir)? {
            let entry = entry?;
            if !self.is_temp_name(&entry.file_name()) || !entry.file_type()?.is_file() {
                continue;
            }
            if now
                .duration_since(entry.metadata()?.modified()?)
                .is_ok_and(|age| age > STALE_TEMP_AGE)
            {
                let path = entry.path();
                match fs::remove_file(&path) {
                    Ok(()) => engine_info!(
                        "[archive-publication] removed stale temp path={}",
                        path.display()
                    ),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => engine_warn!(
                        "[archive-publication] could not remove stale temp path={} reason={error}",
                        path.display()
                    ),
                }
            }
        }
        Ok(())
    }

    fn keep_previous(
        &self,
        file: File,
        current: ArchiveTargetObservation,
        after_sync: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<PathBuf> {
        let mut previous = BufReader::new(file);
        let mut candidates = Vec::new();
        for entry in fs::read_dir(self.dir)? {
            let entry = entry?;
            if self.is_kept_name(entry.file_name().as_encoded_bytes())
                && entry.file_type()?.is_file()
            {
                candidates.push(entry.path());
            }
        }
        candidates.sort();
        for path in candidates {
            if same_contents(&mut previous, &path, current.len)? {
                return Ok(path);
            }
        }

        previous.rewind()?;
        let stamp = archive_index_stamp(&mut previous)?
            .unwrap_or_else(|| DateTime::<Utc>::from(current.modified))
            .format("%Y%m%dT%H%M%SZ")
            .to_string();
        let kept_path = |number| {
            let mut name = self.kept_prefix.clone();
            name.push(&stamp);
            if number > 1 {
                name.push(format!("-{number}"));
            }
            name.push(".md");
            self.dir.join(name)
        };
        let mut number = 1_u64;
        let mut path = kept_path(number);
        while path.symlink_metadata().is_ok() {
            number += 1;
            path = kept_path(number);
        }
        let mut temp = stage_atomic_file(&path)?;
        previous.rewind()?;
        io::copy(&mut previous, &mut temp)?;
        temp.as_file().sync_all()?;
        after_sync()?;
        loop {
            match temp.persist_noclobber(&path) {
                Ok(_) => return Ok(path),
                Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                    temp = error.file;
                    number += 1;
                    path = kept_path(number);
                }
                Err(error) => return Err(error.error),
            }
        }
    }
}

fn same_contents(previous: &mut BufReader<File>, path: &Path, len: u64) -> io::Result<bool> {
    let mut kept = match File::open(path).and_then(|file| Ok((file.metadata()?.len(), file))) {
        Ok((kept_len, file)) if kept_len == len => BufReader::new(file),
        Ok(_) => return Ok(false),
        Err(error) => {
            engine_warn!(
                "[archive-publication] skipping unreadable kept archive path={} reason={error}",
                path.display()
            );
            return Ok(false);
        }
    };
    previous.rewind()?;
    loop {
        let left = previous.fill_buf()?;
        let right = match kept.fill_buf() {
            Ok(bytes) => bytes,
            Err(error) => {
                engine_warn!(
                    "[archive-publication] skipping unreadable kept archive path={} reason={error}",
                    path.display()
                );
                return Ok(false);
            }
        };
        let count = left.len().min(right.len());
        if count == 0 {
            return Ok(left.is_empty() && right.is_empty());
        }
        if left[..count] != right[..count] {
            return Ok(false);
        }
        previous.consume(count);
        kept.consume(count);
    }
}

fn archive_index_stamp(previous: &mut BufReader<File>) -> io::Result<Option<DateTime<Utc>>> {
    let mut line = Vec::new();
    let mut header = false;
    let mut in_index = false;
    let mut until = None;
    let mut fetched_to = None;
    let mut stamp = None;
    while previous.read_until(b'\n', &mut line)? != 0 {
        let bytes = line.trim_ascii_end();
        match bytes {
            b"===== ARCHIVE INDEX =====" => {
                header = true;
                in_index = true;
                until = None;
                fetched_to = None;
                stamp = None;
            }
            b"===== INDEX END =====" if in_index => {
                stamp = until.or(fetched_to);
                in_index = false;
                header = false;
            }
            b"doc | line | fetched_utc | priority | signal_key | title" => header = false,
            _ if header => {
                let parse = |value: &[u8]| {
                    DateTime::parse_from_rfc3339(std::str::from_utf8(value).ok()?)
                        .ok()
                        .map(|date| date.with_timezone(&Utc))
                };
                if let Some(value) = bytes.strip_prefix(b"window_until_utc: ") {
                    until = parse(value);
                } else if let Some(value) = bytes.strip_prefix(b"fetched_to: ") {
                    fetched_to = parse(value);
                }
            }
            _ if !bytes.is_empty() => stamp = None,
            _ => {}
        }
        line.clear();
    }
    Ok(stamp)
}

#[cfg(test)]
#[path = "archive_publication_tests.rs"]
mod tests;
