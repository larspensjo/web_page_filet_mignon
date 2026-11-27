use chrono::{DateTime, Utc};
use harvester_core::morning_archive::{BatchArchiveOutcome, BatchArchiveStatus};
use harvester_engine::format_archive_timestamp;
use std::{fmt::Write, path::PathBuf};

const COST_BASIS: &str = "openai_price_table";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Exported,
    NothingNew,
    CycleFailed,
    ExportFailed,
    Locked,
    UsageError,
}

impl Status {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Exported => "exported",
            Self::NothingNew => "nothing_new",
            Self::CycleFailed => "cycle_failed",
            Self::ExportFailed => "export_failed",
            Self::Locked => "locked",
            Self::UsageError => "usage_error",
        }
    }

    pub(crate) fn exit_code(self) -> i32 {
        match self {
            Self::Exported => 0,
            Self::NothingNew => 10,
            Self::CycleFailed => 20,
            Self::ExportFailed => 21,
            Self::Locked => 22,
            Self::UsageError => 2,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ArchiveResult {
    pub status: Status,
    pub reason: Option<&'static str>,
    pub since_utc: Option<DateTime<Utc>>,
    pub cost_microdollars: Option<u64>,
    pub failed_sources: usize,
    pub lock_holder: Option<String>,
    articles: usize,
    path: Option<PathBuf>,
    previous_kept: Option<PathBuf>,
    previous_superseded: bool,
    until_utc: Option<DateTime<Utc>>,
    backlog_articles: usize,
    excused_articles: usize,
}

impl ArchiveResult {
    pub(crate) fn new(status: Status, reason: Option<&'static str>) -> Self {
        Self {
            status,
            reason,
            articles: 0,
            path: None,
            previous_kept: None,
            previous_superseded: false,
            cost_microdollars: None,
            since_utc: None,
            until_utc: None,
            failed_sources: 0,
            backlog_articles: 0,
            excused_articles: 0,
            lock_holder: None,
        }
    }

    pub(crate) fn from_outcome(outcome: &BatchArchiveOutcome) -> Self {
        let status = match outcome.status {
            BatchArchiveStatus::Exported => Status::Exported,
            BatchArchiveStatus::NothingNew => Status::NothingNew,
            BatchArchiveStatus::CycleFailed => Status::CycleFailed,
            BatchArchiveStatus::ExportFailed => Status::ExportFailed,
            BatchArchiveStatus::UsageError => Status::UsageError,
        };
        Self {
            articles: outcome.doc_count,
            path: outcome.path.clone(),
            previous_kept: outcome.previous_kept.clone(),
            previous_superseded: outcome.previous_superseded,
            until_utc: outcome.window.map(|window| window.until_utc()),
            failed_sources: outcome.failed_sources,
            backlog_articles: outcome.backlog_articles,
            excused_articles: outcome.excused_articles,
            ..Self::new(status, outcome.reason.map(|reason| reason.as_str()))
        }
    }

    pub(crate) fn line(&self) -> String {
        let json = |value: Option<&str>| serde_json::to_string(&value).expect("JSON string");
        let path = self.path.as_deref().map(|path| path.to_string_lossy());
        let previous = self
            .previous_kept
            .as_deref()
            .map(|path| path.to_string_lossy());
        let since = self.since_utc.map(format_archive_timestamp);
        let until = self.until_utc.map(format_archive_timestamp);
        let cost = self.cost_microdollars.map_or_else(
            || "null".to_string(),
            |cost| format!("{}.{:06}", cost / 1_000_000, cost % 1_000_000),
        );
        let line = format!(
            concat!(
                r#"HARVESTER_RESULT {{"v":1,"status":{},"articles":{},"path":{},"#,
                r#""previous_kept":{},"previous_superseded":{},"cost_usd":{},"cost_basis":{},"#,
                r#""archive_schema":2,"since_utc":{},"until_utc":{},"failed_sources":{},"#,
                r#""backlog_articles":{},"excused_articles":{},"lock_holder":{},"reason":{}}}"#,
            ),
            json(Some(self.status.as_str())),
            self.articles,
            json(path.as_deref()),
            json(previous.as_deref()),
            self.previous_superseded,
            cost,
            json(Some(COST_BASIS)),
            json(since.as_deref()),
            json(until.as_deref()),
            self.failed_sources,
            self.backlog_articles,
            self.excused_articles,
            json(self.lock_holder.as_deref()),
            json(self.reason),
        );
        let mut ascii = String::with_capacity(line.len());
        for ch in line.chars() {
            if ch.is_ascii() {
                ascii.push(ch);
            } else {
                for unit in ch.encode_utf16(&mut [0; 2]).iter() {
                    write!(ascii, "\\u{unit:04x}").expect("write to String");
                }
            }
        }
        ascii
    }
}

#[cfg(test)]
mod tests;
