use super::ArchiveReceipts;
use crate::archive_publication::{ArchivePublication, ArchivePublicationObserver, PublishStep};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

pub(super) struct PublicationProgress {
    pub request_id: u64,
    pub last_step_at: Instant,
    pub can_still_publish: bool,
    pub previous: Option<ArchivePublication>,
}

/// Owns the interval during which a worker can still replace the target.
pub(super) struct PublicationJob {
    receipts: Arc<Mutex<ArchiveReceipts>>,
    request_id: u64,
    observer: Option<ArchivePublicationObserver>,
}

impl PublicationJob {
    pub(super) fn start(
        receipts: Arc<Mutex<ArchiveReceipts>>,
        request_id: u64,
        observer: Option<ArchivePublicationObserver>,
    ) -> Self {
        receipts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .publication = Some(PublicationProgress {
            request_id,
            last_step_at: Instant::now(),
            can_still_publish: true,
            previous: None,
        });
        Self {
            receipts,
            request_id,
            observer,
        }
    }

    pub(super) fn observe(&self, step: PublishStep, publication: &ArchivePublication) {
        self.update(|progress| match step {
            PublishStep::KeptPrevious => progress.previous = Some(publication.clone()),
            PublishStep::ReplacedTarget => progress.can_still_publish = false,
            _ => {}
        });
        if let Some(observer) = &self.observer {
            observer(step);
        }
    }

    fn update(&self, update: impl FnOnce(&mut PublicationProgress)) {
        let mut receipts = self
            .receipts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(progress) = receipts
            .publication
            .as_mut()
            .filter(|progress| progress.request_id == self.request_id)
        {
            update(progress);
            progress.last_step_at = Instant::now();
        }
    }
}

impl Drop for PublicationJob {
    fn drop(&mut self) {
        self.update(|progress| progress.can_still_publish = false);
    }
}
