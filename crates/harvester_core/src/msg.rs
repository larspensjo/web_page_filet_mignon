use std::path::PathBuf;

use harvester_engine::ExtractedLink;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// User edited the URL input box (debounced text).
    InputChanged(String),
    /// User submitted the current URL input for ingestion.
    UrlsSubmitted,
    /// Restore previously completed jobs from persisted state.
    RestoreCompletedJobs(Vec<crate::CompletedJobSnapshot>),
    /// User clicked Stop/Finish.
    StopFinishClicked,
    /// User clicked Archive.
    ArchiveClicked,
    /// UI/render tick to coalesce rendering.
    Tick,
    /// Engine progress for a job.
    JobProgress {
        job_id: crate::JobId,
        stage: crate::Stage,
        tokens: Option<u32>,
        bytes: Option<u64>,
        content_preview: Option<String>,
    },
    /// Engine completion for a job.
    JobDone {
        job_id: crate::JobId,
        result: crate::JobResultKind,
        content_preview: Option<String>,
        extracted_links: Vec<ExtractedLink>,
    },
    LinkToggleRequested {
        job_id: crate::JobId,
        link_index: u32,
        checked: bool,
    },
    LinkDownloadStarted {
        job_id: crate::JobId,
        link_index: u32,
    },
    LinkDownloadCompleted {
        job_id: crate::JobId,
        link_index: u32,
        path: PathBuf,
    },
    LinkDownloadFailed {
        job_id: crate::JobId,
        link_index: u32,
        error: String,
    },
    LinkDeleted {
        job_id: crate::JobId,
        link_index: u32,
    },
    /// User selected a job from the tree view.
    JobSelected { job_id: crate::JobId },
    /// User dragged the splitter to resize the left panels.
    SplitterMoved { desired_left_width_px: i32 },
    /// Window was resized.
    WindowResized { window_width: i32 },
    /// Fallback for placeholder wiring.
    NoOp,
}
