//! Reads the consumer's sidecar and the target's authoritative archive bounds.

use chrono::{DateTime, Utc};
use engine_logging::{engine_info, engine_warn};
use harvester_core::morning_archive::{
    ArchiveFileStamp, ArchiveIndex, ExcusedArticle, ObservedArchive, SidecarObservation,
    TargetObservation,
};
use std::{
    fs::File,
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
};

pub fn observe_archive_target(target: &Path) -> TargetObservation {
    TargetObservation {
        sidecar: observe_sidecar(target),
        archive: observe_archive(target),
    }
}

fn observe_sidecar(target: &Path) -> SidecarObservation {
    let mut path = target.as_os_str().to_owned();
    path.push(".processed");
    let path = PathBuf::from(path);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return SidecarObservation::Missing
        }
        Err(error) => {
            engine_warn!(
                "[morning-archive] sidecar={} reason={error}",
                path.display()
            );
            return SidecarObservation::Unreadable;
        }
    };
    let parsed = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
    match parsed.as_ref().and_then(|record| record.get("until_utc")) {
        Some(serde_json::Value::Null) => SidecarObservation::Parsed(None),
        Some(serde_json::Value::String(until)) => parse(until)
            .map(|until| SidecarObservation::Parsed(Some(until)))
            .unwrap_or(SidecarObservation::Unparseable),
        _ => SidecarObservation::Unparseable,
    }
}

fn observe_archive(target: &Path) -> ObservedArchive {
    let file = match File::open(target) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return ObservedArchive::Absent,
        Err(error) => {
            engine_warn!(
                "[morning-archive] target={} cannot read existing archive: {error}",
                target.display()
            );
            return ObservedArchive::Present {
                stamp: None,
                index: ArchiveIndex::Corrupt,
            };
        }
    };
    let stamp = file.metadata().ok().and_then(|meta| {
        Some(ArchiveFileStamp {
            len: meta.len(),
            modified: meta.modified().ok()?,
        })
    });
    let index = read_archive_index(BufReader::new(file)).unwrap_or_else(|error| {
        engine_warn!(
            "[morning-archive] target={} cannot read archive index: {error}",
            target.display()
        );
        ArchiveIndex::Corrupt
    });
    ObservedArchive::Present { stamp, index }
}

const COLUMNS: &str = "doc | line | fetched_utc | priority | signal_key | title";

#[derive(Clone, Copy)]
enum Position {
    BetweenDocs,
    DocHeader,
    DocBody,
    IndexHeader,
    IndexRows,
    End,
}

#[derive(Default)]
struct IndexHeader {
    schema: Option<u8>,
    doc_count: Option<usize>,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    fetched_from: Option<DateTime<Utc>>,
    fetched_to: Option<DateTime<Utc>>,
    invalid: bool,
}

impl IndexHeader {
    fn record(&mut self, key: &str, value: &str) {
        match key {
            "export_schema" => self.schema = value.parse().ok(),
            "doc_count" => self.doc_count = value.parse().ok(),
            "fetched_from" => self.fetched_from = parse(value),
            "fetched_to" => self.fetched_to = parse(value),
            "window_since_utc" => {
                self.invalid |= self.since.is_some();
                self.since = parse(value);
                self.invalid |= self.since.is_none();
            }
            "window_until_utc" => {
                self.invalid |= self.until.is_some();
                self.until = parse(value);
                self.invalid |= self.until.is_none();
            }
            _ => {}
        }
    }

    fn finish(self, rows: usize, docs: usize) -> ArchiveIndex {
        if self.invalid
            || self.schema != Some(2)
            || self.doc_count != Some(rows)
            || rows != docs
            || self
                .since
                .is_some_and(|since| self.until.is_none_or(|until| since > until))
        {
            return ArchiveIndex::Corrupt;
        }
        ArchiveIndex::Schema2 {
            window_since_utc: self.since,
            window_until_utc: self.until,
            fetched_from: self.fetched_from,
            fetched_to: self.fetched_to,
        }
    }
}

fn parse(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

pub(crate) fn read_archive_index(reader: impl BufRead) -> io::Result<ArchiveIndex> {
    read_archive_index_and_count(reader).map(|(index, _)| index)
}

/// Return the validated document count when the target carries the pending upper bound.
pub fn published_article_count(target: &Path, pending_until: DateTime<Utc>) -> Option<usize> {
    let file = File::open(target).ok()?;
    let (index, count) = read_archive_index_and_count(BufReader::new(file)).ok()?;
    (index.window_until() == Some(pending_until)).then_some(count)
}

fn read_archive_index_and_count(reader: impl BufRead) -> io::Result<(ArchiveIndex, usize)> {
    use Position as P;
    let mut position = P::BetweenDocs;
    let mut doc_lines = Vec::new();
    let mut versioned_doc = false;
    let mut fetched_from = None;
    let mut fetched_to = None;
    let mut index = IndexHeader::default();
    let mut rows = 0;
    for (offset, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim_end();
        position = match (position, line) {
            (P::BetweenDocs, "===== DOC START =====") => {
                doc_lines.push(offset + 1);
                P::DocHeader
            }
            (P::BetweenDocs, "===== ARCHIVE INDEX =====") => P::IndexHeader,
            (P::BetweenDocs, "") => P::BetweenDocs,
            (P::DocHeader, "") => P::DocBody,
            (P::DocBody, "===== DOC END =====") => P::BetweenDocs,
            (P::IndexHeader, COLUMNS) => P::IndexRows,
            (P::IndexRows, "===== INDEX END =====") => P::End,
            (P::End, "") => P::End,
            (
                _,
                "===== DOC START ====="
                | "===== DOC END ====="
                | "===== ARCHIVE INDEX ====="
                | "===== INDEX END =====",
            ) => return Ok((ArchiveIndex::Corrupt, 0)),
            (P::DocHeader, line) => {
                if line.starts_with("export_schema: ") {
                    versioned_doc = true;
                }
                if let Some(date) = line.strip_prefix("fetched_utc: ").and_then(parse) {
                    fetched_from =
                        Some(fetched_from.map_or(date, |old: DateTime<Utc>| old.min(date)));
                    fetched_to = Some(fetched_to.map_or(date, |old: DateTime<Utc>| old.max(date)));
                }
                P::DocHeader
            }
            (P::DocBody, _) => P::DocBody,
            (P::IndexHeader, line) => {
                let Some((key, value)) = line.split_once(": ") else {
                    return Ok((ArchiveIndex::Corrupt, 0));
                };
                index.record(key, value);
                P::IndexHeader
            }
            (P::IndexRows, line) => {
                let Some((number, line)) = parse_row(line) else {
                    return Ok((ArchiveIndex::Corrupt, 0));
                };
                if number != rows + 1 || doc_lines.get(rows) != Some(&line) {
                    return Ok((ArchiveIndex::Corrupt, 0));
                }
                rows += 1;
                P::IndexRows
            }
            _ => return Ok((ArchiveIndex::Corrupt, 0)),
        };
    }
    let index = match position {
        P::End => index.finish(rows, doc_lines.len()),
        P::BetweenDocs if !versioned_doc && !doc_lines.is_empty() => ArchiveIndex::Schema1 {
            fetched_from,
            fetched_to,
        },
        _ => ArchiveIndex::Corrupt,
    };
    Ok((index, rows))
}

fn parse_row(row: &str) -> Option<(usize, usize)> {
    let mut fields = row.splitn(6, " | ");
    let number = fields.next()?.parse().ok()?;
    let line = fields.next()?.parse().ok()?;
    (fields.count() == 4).then_some((number, line))
}

pub(crate) fn log_excused_articles(target: &Path, articles: &[ExcusedArticle], blocked: bool) {
    for article in articles {
        let stages = article
            .stages
            .iter()
            .map(|stage| stage.as_str())
            .collect::<Vec<_>>()
            .join(",");
        engine_info!(
            "[morning-archive] excused url={} stage={} strikes=2 target={} blocked={blocked}",
            article.url,
            stages,
            target.display()
        );
    }
}
