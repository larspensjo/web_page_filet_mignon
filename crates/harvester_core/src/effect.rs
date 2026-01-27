use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    EnqueueUrl {
        job_id: crate::JobId,
        url: String,
    },
    StartSession,
    StopFinish {
        policy: StopPolicy,
    },
    ArchiveRequested,
    DownloadLinkedPage {
        job_id: crate::JobId,
        link_index: u32,
        url: String,
    },
    DeleteLinkedPage {
        job_id: crate::JobId,
        link_index: u32,
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopPolicy {
    Finish,
    Immediate,
}
