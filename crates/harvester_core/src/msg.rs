#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// User pasted URLs into the input box.
    UrlsPasted(String),
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
    },
    /// Engine completion for a job.
    JobDone {
        job_id: crate::JobId,
        result: crate::JobResultKind,
        content_preview: Option<String>,
    },
    /// Fallback for placeholder wiring.
    NoOp,
}
