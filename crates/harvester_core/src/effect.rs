#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    EnqueueUrl { job_id: crate::JobId, url: String },
    StartSession,
    StopFinish { policy: StopPolicy },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopPolicy {
    Finish,
    Immediate,
}
