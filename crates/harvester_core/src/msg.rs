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
    },
    /// User selected a job from the tree view.
    JobSelected { job_id: crate::JobId },
    /// Fallback for placeholder wiring.
    NoOp,
}
