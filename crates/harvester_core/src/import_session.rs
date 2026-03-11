//! Reducer-owned state for the imported-corpus workflow.

use std::path::PathBuf;

use harvester_engine::ImportedArchiveRef;

/// Lifecycle phase of an import session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImportPhase {
    #[default]
    Idle,
    /// Scanning the directory and importing files.
    Importing,
    /// Import completed; downstream work may be in flight.
    Complete,
    /// Import failed at the directory/setup level.
    Failed,
}

/// Reducer-owned state for one import request lifetime.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportSessionState {
    /// Monotonically increasing ID for the current authoritative request.
    /// Completions carrying a different ID are stale and must be ignored.
    pub request_id: u64,
    /// Phase of the active or most-recently-completed import.
    pub phase: ImportPhase,
    /// Source directory for the current or most-recently-completed import.
    pub source_dir: Option<PathBuf>,
    /// Archive refs produced by the authoritative import.
    /// Empty until `ImportSavedWebpagesCompleted` arrives for the current request_id.
    pub imported_entries: Vec<ImportedArchiveRef>,
    /// Count of successfully persisted imports.
    pub imports_completed: usize,
    /// Count of per-file import failures.
    pub imports_failed: usize,
    /// Number of duplicate canonical URLs encountered.
    pub duplicate_url_count: usize,
    /// Number of duplicate content hashes encountered.
    pub duplicate_content_count: usize,
    /// Accumulated warning messages.
    pub warnings: Vec<String>,
    /// Terminal failure reason (only set when phase == Failed).
    pub failure_reason: Option<String>,
}

impl ImportSessionState {
    /// Advance to a new authoritative import request.
    pub fn start_import(&mut self, request_id: u64, source_dir: PathBuf) {
        self.request_id = request_id;
        self.phase = ImportPhase::Importing;
        self.source_dir = Some(source_dir);
        self.imported_entries.clear();
        self.imports_completed = 0;
        self.imports_failed = 0;
        self.duplicate_url_count = 0;
        self.duplicate_content_count = 0;
        self.warnings.clear();
        self.failure_reason = None;
    }

    /// Returns true when `request_id` is the current authoritative ID.
    pub fn is_authoritative(&self, request_id: u64) -> bool {
        self.request_id == request_id && self.phase == ImportPhase::Importing
    }

    /// Reset to idle / empty state.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}
