use super::ui::tree_item_ids::{decode_tree_item_id, TreeItemKind};
use super::{triage_marker_for_priority, SharedState};
use commanductui::types::TreeItemMarkerKind;
use commanductui::{TreeItemId, UiStateProvider, WindowId};
use harvester_core::{LeftTab, LinkDownloadState};
use std::sync::{Arc, Mutex};

pub(super) struct AppUiStateProvider {
    shared: Arc<Mutex<SharedState>>,
}

impl AppUiStateProvider {
    pub(super) fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self { shared }
    }
}

impl UiStateProvider for AppUiStateProvider {
    fn is_tree_item_new(&self, _window_id: WindowId, _item_id: TreeItemId) -> bool {
        false
    }

    fn tree_item_marker(&self, _window_id: WindowId, item_id: TreeItemId) -> TreeItemMarkerKind {
        match decode_tree_item_id(item_id) {
            TreeItemKind::Job { job_id } => {
                let guard = self.shared.lock().unwrap();
                if guard.state.left_tab() != LeftTab::TriageResults {
                    return TreeItemMarkerKind::None;
                }
                if let Some(result) = guard.state.triage_result_for_job(job_id) {
                    return triage_marker_for_priority(result.priority);
                }
                TreeItemMarkerKind::None
            }
            TreeItemKind::Link { job_id, link_index } => {
                let guard = self.shared.lock().unwrap();
                if let Some((download_state, age_suspect)) =
                    guard.state.link_state(job_id, link_index)
                {
                    return match download_state {
                        LinkDownloadState::Downloaded { .. } => TreeItemMarkerKind::Green,
                        LinkDownloadState::Downloading => TreeItemMarkerKind::Purple,
                        LinkDownloadState::Failed { .. } => TreeItemMarkerKind::Red,
                        LinkDownloadState::NotDownloaded if age_suspect => {
                            TreeItemMarkerKind::Yellow
                        }
                        _ => TreeItemMarkerKind::None,
                    };
                }
                TreeItemMarkerKind::None
            }
            _ => TreeItemMarkerKind::None,
        }
    }
}
