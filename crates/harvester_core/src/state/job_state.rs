use super::{
    build_link_rows, normalize_extracted_link, JobId, JobOrigin, JobResultKind, LinkDownloadState,
    LinkRecord, LinkSnapshotRecord, Stage, MAX_EXTRACTED_LINKS,
};
use crate::url_age::guess_age_from_url;
use crate::view_model::JobRowView;
use harvester_engine::ExtractedLink;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct JobState {
    pub(super) url: String,
    pub(super) stage: Stage,
    pub(super) outcome: Option<JobResultKind>,
    pub(super) tokens: Option<u32>,
    pub(super) bytes: Option<u64>,
    pub(super) content_preview: Option<String>,
    pub(super) preview_quality: Option<PreviewQuality>,
    pub(super) links: Vec<LinkRecord>,
    pub(super) origin: JobOrigin,
    pub(super) fetched_utc: Option<chrono::DateTime<chrono::Utc>>,
}

impl JobState {
    pub(super) fn to_view(&self, id: JobId, is_since_checkpoint: bool) -> JobRowView {
        let links = build_link_rows(&self.links);
        let downloaded_link_count = self
            .links
            .iter()
            .filter(|link| matches!(link.download_state, LinkDownloadState::Downloaded { .. }))
            .count();
        JobRowView {
            job_id: id,
            url: self.url.clone(),
            stage: self.stage,
            outcome: self.outcome.clone(),
            tokens: self.tokens,
            bytes: self.bytes,
            link_count: self.links.len(),
            downloaded_link_count,
            links,
            origin: self.origin.clone(),
            triage_annotation: None,
            has_summary: false,
            summary_title: None,
            filter_status: None,
            has_analysis: false,
            is_since_checkpoint,
        }
    }

    #[allow(dead_code)]
    pub(super) fn content_preview(&self) -> Option<&str> {
        self.content_preview.as_deref()
    }

    pub(super) fn set_preview_content(&mut self, content: String) {
        self.preview_quality = Some(PreviewQuality::from_markdown(&content));
        self.content_preview = Some(content);
    }

    pub(super) fn clear_preview_content(&mut self) {
        self.preview_quality = None;
        self.content_preview = None;
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
                download_state: LinkDownloadState::NotDownloaded,
                age_estimate: guess_age_from_url(&canonical),
            });
        }
    }

    pub(super) fn apply_link_snapshots(&mut self, snapshots: &[LinkSnapshotRecord]) {
        for snapshot in snapshots {
            if let Some(path) = snapshot.downloaded_path.as_ref() {
                let canonical = normalize_extracted_link(&snapshot.url);
                if canonical.is_empty() {
                    continue;
                }
                if let Some(record) = self.links.iter_mut().find(|record| record.url == canonical) {
                    record.download_state = LinkDownloadState::Downloaded {
                        path: PathBuf::from(path),
                    };
                }
            }
        }
    }

    #[allow(dead_code)]
    fn find_link_mut(&mut self, link_index: u32) -> Option<&mut LinkRecord> {
        self.links
            .iter_mut()
            .find(|record| record.index == link_index)
    }

    #[allow(dead_code)]
    pub(super) fn mark_link_download_requested(&mut self, link_index: u32) {
        if let Some(record) = self.find_link_mut(link_index) {
            record.download_state = LinkDownloadState::Downloading;
        }
    }

    #[allow(dead_code)]
    pub(super) fn mark_link_download_completed(&mut self, link_index: u32, path: PathBuf) {
        if let Some(record) = self.find_link_mut(link_index) {
            record.download_state = LinkDownloadState::Downloaded { path };
        }
    }

    #[allow(dead_code)]
    pub(super) fn mark_link_download_failed(&mut self, link_index: u32, error: String) {
        if let Some(record) = self.find_link_mut(link_index) {
            record.download_state = LinkDownloadState::Failed { error };
        }
    }

    #[allow(dead_code)]
    pub(super) fn mark_link_deleted(&mut self, link_index: u32) {
        if let Some(record) = self.find_link_mut(link_index) {
            record.download_state = LinkDownloadState::NotDownloaded;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PreviewQuality {
    pub(super) heading_count: usize,
    pub(super) link_density: f64,
}

impl Default for PreviewQuality {
    fn default() -> Self {
        Self {
            heading_count: 0,
            link_density: 0.0,
        }
    }
}

impl PreviewQuality {
    const NAV_HEAVY_THRESHOLD: f64 = 0.3;

    pub(super) fn from_markdown(content: &str) -> Self {
        let heading_count = content
            .lines()
            .filter(|line| line.trim_start().starts_with('#'))
            .count();
        let link_count = content
            .split('[')
            .skip(1)
            .filter(|segment| segment.contains("]("))
            .count();
        let word_count = content.split_whitespace().count();
        let link_density = if word_count > 0 {
            link_count as f64 / word_count as f64
        } else {
            0.0
        };
        Self {
            heading_count,
            link_density,
        }
    }

    pub(super) fn nav_heavy(&self) -> bool {
        self.link_density > Self::NAV_HEAVY_THRESHOLD
    }
}
