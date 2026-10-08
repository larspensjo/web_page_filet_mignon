use super::{
    build_link_rows, normalize_extracted_link, JobId, JobOrigin, JobResultKind, LinkRecord, Stage,
    MAX_EXTRACTED_LINKS,
};
use crate::view_model::JobRowView;
use harvester_engine::ExtractedLink;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct JobState {
    pub(super) url: String,
    /// Canonical archive identity, computed when a reducer creates or changes the URL.
    pub(super) archive_url_key: String,
    pub(super) stage: Stage,
    pub(super) outcome: Option<JobResultKind>,
    pub(super) tokens: Option<u32>,
    pub(super) bytes: Option<u64>,
    pub(super) links: Vec<LinkRecord>,
    pub(super) origin: JobOrigin,
    pub(super) fetched_utc: Option<chrono::DateTime<chrono::Utc>>,
}

impl JobState {
    #[cfg(test)]
    pub(super) fn set_url(&mut self, url: String) {
        self.archive_url_key = harvester_engine::archive_url_key(&url);
        self.url = url;
    }

    pub(super) fn archive_url_key(&self) -> std::borrow::Cow<'_, str> {
        if self.archive_url_key.is_empty() {
            // Directly constructed test jobs use Default for this derived field.
            harvester_engine::archive_url_key(&self.url).into()
        } else {
            self.archive_url_key.as_str().into()
        }
    }

    pub(super) fn to_view(&self, id: JobId, is_since_checkpoint: bool) -> JobRowView {
        let links = build_link_rows(&self.links);
        JobRowView {
            job_id: id,
            url: self.url.clone(),
            stage: self.stage,
            outcome: self.outcome.clone(),
            tokens: self.tokens,
            bytes: self.bytes,
            link_count: self.links.len(),
            links,
            origin: self.origin.clone(),
            triage_annotation: None,
            has_summary: false,
            summary_title: None,
            summary_tokens: None,
            filter_status: None,
            has_analysis: false,
            is_since_checkpoint,
        }
    }

    #[allow(dead_code)]
    pub(super) fn links(&self) -> &[LinkRecord] {
        &self.links
    }

    #[allow(dead_code)]
    pub(super) fn clear_links(&mut self) {
        self.links.clear();
    }

    pub(super) fn attach_extracted_links(&mut self, links: Vec<ExtractedLink>) {
        self.links.clear();
        let mut seen = HashSet::new();
        for (idx, link) in links.into_iter().enumerate() {
            if self.links.len() >= MAX_EXTRACTED_LINKS {
                break;
            }
            let canonical = normalize_extracted_link(&link.url);
            if canonical.is_empty() {
                continue;
            }
            if !seen.insert(canonical.clone()) {
                continue;
            }
            self.links.push(LinkRecord {
                index: idx as u32,
                url: canonical.clone(),
                anchor_text: link.text,
                kind: link.kind,
            });
        }
    }
}
