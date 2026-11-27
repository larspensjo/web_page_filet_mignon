//! Durable portfolio state and pure morning archive decisions.

use crate::StartupReadinessStatus;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub type PortfolioRecords = BTreeMap<String, PortfolioRecord>;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefingCheckpoint {
    pub since_utc: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub portfolio: PortfolioRecords,
}

impl BriefingCheckpoint {
    pub fn set_since_utc(&mut self, since_utc: Option<String>) {
        self.since_utc = since_utc;
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioRecord {
    pub confirmed_until_utc: Option<DateTime<Utc>>,
    pub last_archive_until_utc: Option<DateTime<Utc>>,
    pub pending_until_utc: Option<DateTime<Utc>>,
    // Confirmed windows retire dated entries. Until then, an unconfirmed span
    // may replay, and an empty selection has no archive to confirm.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub left_out: BTreeMap<String, Option<DateTime<Utc>>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pending_left_out: BTreeMap<String, Option<DateTime<Utc>>>,
}

impl PortfolioRecord {
    pub(crate) fn complete_publication(&mut self, until_utc: DateTime<Utc>) {
        self.last_archive_until_utc = Some(until_utc);
        self.pending_until_utc = None;
        self.left_out.append(&mut self.pending_left_out);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointWrite {
    Unchanged,
    AdvanceTo(DateTime<Utc>),
    Set(DateTime<Utc>),
}

/// Apply a portfolio write to the latest checkpoint without replacing other target records.
pub fn merge_portfolio_record(
    stored: &mut BriefingCheckpoint,
    key: &str,
    record: &PortfolioRecord,
    checkpoint: CheckpointWrite,
) {
    stored.portfolio.insert(key.to_owned(), record.clone());
    let until = match checkpoint {
        CheckpointWrite::Unchanged => return,
        CheckpointWrite::Set(until) => until,
        CheckpointWrite::AdvanceTo(until) => {
            let previous = stored.since_utc.as_deref().and_then(parse_instant);
            if previous.is_some_and(|since| since >= until) {
                return;
            }
            until
        }
    };
    stored.set_since_utc(Some(harvester_engine::format_archive_timestamp(until)));
}

/// The caller supplies an absolute target. Keys follow Windows path spelling,
/// including equivalence of slash, case, and ordinary verbatim path aliases.
pub fn target_key(target: &Path) -> String {
    let path = target.to_string_lossy().replace('/', "\\").to_lowercase();
    let path = if let Some(unc) = path.strip_prefix(r"\\?\unc\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    };
    let prefix = if path.starts_with(r"\\") {
        r"\\"
    } else if path.starts_with('\\') {
        r"\"
    } else {
        ""
    };
    let root_components = if prefix == r"\\" {
        2
    } else if prefix.is_empty() {
        1
    } else {
        0
    };
    let mut parts = Vec::new();
    for part in path.trim_start_matches('\\').split('\\') {
        match part {
            "" | "." => {}
            ".." if parts.len() > root_components => {
                parts.pop();
            }
            ".." => {}
            _ => parts.push(part),
        }
    }
    format!("{prefix}{}", parts.join("\\"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidecarObservation {
    Missing,
    Unreadable,
    Unparseable,
    Parsed(Option<DateTime<Utc>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarStatus {
    Missing,
    Unreadable,
    Unparseable,
    Null,
    Older,
    Equal,
    Future,
    NoLastArchive,
    BeyondLastArchive,
    Seeded,
    Advanced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveFileStamp {
    pub len: u64,
    pub modified: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchiveIndex {
    Schema2 {
        window_since_utc: Option<DateTime<Utc>>,
        window_until_utc: Option<DateTime<Utc>>,
        fetched_from: Option<DateTime<Utc>>,
        fetched_to: Option<DateTime<Utc>>,
    },
    Schema1 {
        fetched_from: Option<DateTime<Utc>>,
        fetched_to: Option<DateTime<Utc>>,
    },
    Corrupt,
}

impl ArchiveIndex {
    pub fn window_until(self) -> Option<DateTime<Utc>> {
        match self {
            Self::Schema2 {
                window_until_utc, ..
            } => window_until_utc,
            _ => None,
        }
    }

    pub fn archive_start(self) -> Option<DateTime<Utc>> {
        match self {
            Self::Schema2 {
                window_until_utc: Some(_),
                window_since_utc,
                ..
            } => window_since_utc,
            Self::Schema2 { fetched_from, .. } | Self::Schema1 { fetched_from, .. } => fetched_from,
            Self::Corrupt => None,
        }
    }

    pub fn fetched_to(self) -> Option<DateTime<Utc>> {
        match self {
            Self::Schema2 { fetched_to, .. } | Self::Schema1 { fetched_to, .. } => fetched_to,
            Self::Corrupt => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObservedArchive {
    Absent,
    Present {
        stamp: Option<ArchiveFileStamp>,
        index: ArchiveIndex,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetObservation {
    pub sidecar: SidecarObservation,
    pub archive: ObservedArchive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviousArchiveDisposition {
    None,
    Replace { observed: ArchiveFileStamp },
    KeepAside,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalArchiveTarget {
    pub path: PathBuf,
    pub disposition: PreviousArchiveDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWindow {
    pub record: PortfolioRecord,
    pub since_utc: DateTime<Utc>,
    pub checkpoint_advance: Option<DateTime<Utc>>,
    pub disposition: PreviousArchiveDisposition,
    pub previous_superseded: bool,
    pub sidecar_status: SidecarStatus,
}

fn parse_instant(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

impl SidecarObservation {
    fn valid_until(self, now: DateTime<Utc>) -> Result<DateTime<Utc>, SidecarStatus> {
        match self {
            Self::Missing => Err(SidecarStatus::Missing),
            Self::Unreadable => Err(SidecarStatus::Unreadable),
            Self::Unparseable => Err(SidecarStatus::Unparseable),
            Self::Parsed(None) => Err(SidecarStatus::Null),
            Self::Parsed(Some(until)) if until > now => Err(SidecarStatus::Future),
            Self::Parsed(Some(until)) => Ok(until),
        }
    }
}

pub fn resolve_window(
    checkpoint: &BriefingCheckpoint,
    key: &str,
    seen: &TargetObservation,
    now: DateTime<Utc>,
) -> Result<ResolvedWindow, BatchArchiveReason> {
    use SidecarStatus as S;
    let mut record = checkpoint.portfolio.get(key).cloned().unwrap_or_default();
    let desktop = checkpoint.since_utc.as_deref().and_then(parse_instant);
    let mut checkpoint_advance = None;
    if let Some(pending) = record.pending_until_utc.take() {
        if matches!(seen.archive, ObservedArchive::Present { index, .. } if index.window_until() == Some(pending))
        {
            record.complete_publication(pending);
            checkpoint_advance = desktop
                .is_none_or(|since| since < pending)
                .then_some(pending);
        }
    }
    record.pending_left_out.clear();
    let sidecar = seen.sidecar.valid_until(now);
    let sidecar_status = match (record.confirmed_until_utc, sidecar) {
        (_, Err(status)) => status,
        (None, Ok(until)) => {
            record.confirmed_until_utc = Some(until);
            S::Seeded
        }
        (Some(since), Ok(until)) if until < since => S::Older,
        (Some(since), Ok(until)) if until == since => S::Equal,
        (_, Ok(_)) if record.last_archive_until_utc.is_none() => S::NoLastArchive,
        (_, Ok(until))
            if record
                .last_archive_until_utc
                .is_some_and(|last| until > last) =>
        {
            S::BeyondLastArchive
        }
        (_, Ok(until)) => {
            record.confirmed_until_utc = Some(until);
            S::Advanced
        }
    };
    let since_utc = record
        .confirmed_until_utc
        .or(checkpoint_advance)
        .or(desktop)
        .ok_or(BatchArchiveReason::NoWindowStart)?;
    record.confirmed_until_utc = Some(since_utc);
    record
        .left_out
        .retain(|_, fetched| fetched.is_none_or(|fetched| fetched >= since_utc));
    let (disposition, previous_superseded) = match seen.archive {
        ObservedArchive::Absent => (PreviousArchiveDisposition::None, false),
        ObservedArchive::Present { stamp, index } => {
            let confirmed = match (index, index.window_until()) {
                (ArchiveIndex::Corrupt, _) => false,
                (_, Some(until)) => since_utc >= until,
                _ => sidecar.is_ok_and(|date| index.fetched_to().is_none_or(|end| date >= end)),
            };
            match stamp.filter(|_| confirmed) {
                Some(observed) => (PreviousArchiveDisposition::Replace { observed }, false),
                None => (
                    PreviousArchiveDisposition::KeepAside,
                    index
                        .archive_start()
                        .is_some_and(|start| since_utc <= start),
                ),
            }
        }
    };
    Ok(ResolvedWindow {
        record,
        since_utc,
        checkpoint_advance,
        disposition,
        previous_superseded,
        sidecar_status,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FailedStage {
    Triage,
    Summary,
    Scoring,
}

impl FailedStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Triage => "triage",
            Self::Summary => "summary",
            Self::Scoring => "scoring",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LeftOutArticle {
    pub url: String,
    pub title: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchArchiveReason {
    NoWindowStart,
    RecordUnreadable,
    RecordNotSaved,
    Interrupted,
    ResultStoreUnavailable,
    SavedResultsUnavailable,
    QuotaHalted,
    TotalFailure,
    ExportWriteFailed,
    CheckpointNotSaved,
}

impl BatchArchiveReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoWindowStart => "no_window_start",
            Self::RecordUnreadable => "record_unreadable",
            Self::RecordNotSaved => "record_not_saved",
            Self::Interrupted => "interrupted",
            Self::ResultStoreUnavailable => "result_store_unavailable",
            Self::SavedResultsUnavailable => "saved_results_unavailable",
            Self::QuotaHalted => "quota_halted",
            Self::TotalFailure => "total_failure",
            Self::ExportWriteFailed => "export_write_failed",
            Self::CheckpointNotSaved => "checkpoint_not_saved",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchArchiveStatus {
    UsageError,
    CycleFailed,
    NothingNew,
    ExportFailed,
    Exported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchArchiveOutcome {
    pub status: BatchArchiveStatus,
    pub reason: Option<BatchArchiveReason>,
    pub left_out: Vec<LeftOutArticle>,
    pub failed_sources: usize,
    pub path: Option<PathBuf>,
    pub doc_count: usize,
    pub previous_kept: Option<PathBuf>,
    pub previous_superseded: bool,
    pub window: Option<harvester_engine::ArchiveWindow>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GateFacts {
    pub interrupted: bool,
    pub result_store_refused: bool,
    pub readiness: StartupReadinessStatus,
    pub quota_halted: bool,
    pub sources_scanned: usize,
    pub failed_sources: usize,
}

pub(crate) fn morning_block_reason(facts: GateFacts) -> Option<BatchArchiveReason> {
    use BatchArchiveReason as R;
    if facts.interrupted {
        Some(R::Interrupted)
    } else if facts.result_store_refused {
        Some(R::ResultStoreUnavailable)
    } else if facts.readiness != StartupReadinessStatus::Ready {
        Some(R::SavedResultsUnavailable)
    } else if facts.quota_halted {
        Some(R::QuotaHalted)
    } else if facts.sources_scanned > 0 && facts.sources_scanned == facts.failed_sources {
        Some(R::TotalFailure)
    } else {
        None
    }
}
