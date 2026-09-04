use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{JobId, JobListMode, Msg, ReadingPaneMode, TrendCategory, WorkspaceView};

/// The deliberately restricted frontend vocabulary.
///
/// JSON uses adjacently tagged objects (`{"type":"PollSources"}` or
/// `{"type":"SelectJob","payload":{"job_id":1}}`). Unknown fields are
/// rejected rather than ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", deny_unknown_fields)]
pub enum UiIntent {
    SelectJob {
        job_id: JobId,
    },
    SetWorkspaceView {
        view: WorkspaceView,
    },
    SetJobListMode {
        mode: JobListMode,
    },
    SetJobsSearchQuery {
        text: String,
    },
    ClearJobsSearch,
    RevealJobsSearch,
    SetTrendCategory {
        category: TrendCategory,
    },
    TrendsViewOpened,
    DismissRunFinishedNotice,
    PollSources,
    PollIndirectLinks,
    RunPipeline,
    StopOrFinish,
    OpenSelectedInBrowser,
    OpenExtractedLink {
        job_id: JobId,
        link_index: u32,
    },
    SetReadingPaneMode {
        mode: ReadingPaneMode,
    },
    OpenArchiveDialog,
    SubmitArchiveDialog {
        request_id: u64,
        basename: String,
        set_checkpoint: bool,
        use_summaries: bool,
        use_signal_candidates: bool,
    },
    CancelArchiveDialog,
    ToggleSignalCandidateExclusion {
        signal_key: String,
    },
    SetUrlInput {
        text: String,
    },
    SubmitUrls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntentContext {
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentEffect {
    Dispatch(Msg),
    Host(HostAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostAction {
    CancelArchiveDialog,
}

impl UiIntent {
    /// Total and infallible: every frontend action is mapped in this exhaustive match.
    pub fn into_effect(self, context: &IntentContext) -> IntentEffect {
        use UiIntent::*;
        let message = match self {
            SelectJob { job_id } => Msg::JobSelected { job_id },
            SetWorkspaceView { view } => Msg::WorkspaceViewSet { view },
            SetJobListMode { mode } => Msg::JobListModeSet { mode },
            SetJobsSearchQuery { text } => Msg::JobsSearchQueryChanged(text),
            ClearJobsSearch => Msg::JobsSearchCleared,
            RevealJobsSearch => Msg::JobsSearchRevealRequested,
            SetTrendCategory { category } => Msg::TrendCategorySelected { category },
            TrendsViewOpened => Msg::TrendsViewOpened,
            DismissRunFinishedNotice => Msg::RunFinishedNoticeDismissed,
            PollSources => Msg::PollSourcesClicked,
            PollIndirectLinks => Msg::PollIndirectLinks,
            RunPipeline => Msg::PipelineRunRequested,
            StopOrFinish => Msg::StopFinishClicked,
            OpenSelectedInBrowser => Msg::OpenInBrowserClicked,
            OpenExtractedLink { job_id, link_index } => {
                Msg::ExtractedLinkOpenRequested { job_id, link_index }
            }
            SetReadingPaneMode { mode } => Msg::ReadingPaneModeSet { mode },
            OpenArchiveDialog => Msg::ArchiveClicked,
            SubmitArchiveDialog {
                request_id,
                basename,
                set_checkpoint,
                use_summaries,
                use_signal_candidates,
            } => Msg::ArchiveDialogSubmitted {
                request_id,
                basename,
                set_checkpoint,
                submitted_at: context.now,
                use_summaries,
                use_signal_candidates,
            },
            CancelArchiveDialog => return IntentEffect::Host(HostAction::CancelArchiveDialog),
            ToggleSignalCandidateExclusion { signal_key } => {
                Msg::ToggleSignalCandidateExclusion { signal_key }
            }
            SetUrlInput { text } => Msg::InputChanged(text),
            SubmitUrls => Msg::UrlsSubmitted,
        };
        IntentEffect::Dispatch(message)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::{HostAction, IntentContext, IntentEffect, UiIntent};
    use crate::{JobListMode, Msg, ReadingPaneMode, TrendCategory, WorkspaceView};

    #[test]
    fn every_ui_intent_maps_to_its_exact_effect() {
        let now = DateTime::parse_from_rfc3339("2026-09-03T12:34:56Z")
            .unwrap()
            .with_timezone(&Utc);
        let context = IntentContext { now };
        let cases = vec![
            (
                UiIntent::SelectJob { job_id: 7 },
                IntentEffect::Dispatch(Msg::JobSelected { job_id: 7 }),
            ),
            (
                UiIntent::SetWorkspaceView {
                    view: WorkspaceView::Blacklist,
                },
                IntentEffect::Dispatch(Msg::WorkspaceViewSet {
                    view: WorkspaceView::Blacklist,
                }),
            ),
            (
                UiIntent::SetJobListMode {
                    mode: JobListMode::Results,
                },
                IntentEffect::Dispatch(Msg::JobListModeSet {
                    mode: JobListMode::Results,
                }),
            ),
            (
                UiIntent::SetJobsSearchQuery {
                    text: "query".into(),
                },
                IntentEffect::Dispatch(Msg::JobsSearchQueryChanged("query".into())),
            ),
            (
                UiIntent::ClearJobsSearch,
                IntentEffect::Dispatch(Msg::JobsSearchCleared),
            ),
            (
                UiIntent::RevealJobsSearch,
                IntentEffect::Dispatch(Msg::JobsSearchRevealRequested),
            ),
            (
                UiIntent::SetTrendCategory {
                    category: TrendCategory::Themes,
                },
                IntentEffect::Dispatch(Msg::TrendCategorySelected {
                    category: TrendCategory::Themes,
                }),
            ),
            (
                UiIntent::TrendsViewOpened,
                IntentEffect::Dispatch(Msg::TrendsViewOpened),
            ),
            (
                UiIntent::DismissRunFinishedNotice,
                IntentEffect::Dispatch(Msg::RunFinishedNoticeDismissed),
            ),
            (
                UiIntent::PollSources,
                IntentEffect::Dispatch(Msg::PollSourcesClicked),
            ),
            (
                UiIntent::PollIndirectLinks,
                IntentEffect::Dispatch(Msg::PollIndirectLinks),
            ),
            (
                UiIntent::RunPipeline,
                IntentEffect::Dispatch(Msg::PipelineRunRequested),
            ),
            (
                UiIntent::StopOrFinish,
                IntentEffect::Dispatch(Msg::StopFinishClicked),
            ),
            (
                UiIntent::OpenSelectedInBrowser,
                IntentEffect::Dispatch(Msg::OpenInBrowserClicked),
            ),
            (
                UiIntent::OpenExtractedLink {
                    job_id: 9,
                    link_index: 3,
                },
                IntentEffect::Dispatch(Msg::ExtractedLinkOpenRequested {
                    job_id: 9,
                    link_index: 3,
                }),
            ),
            (
                UiIntent::SetReadingPaneMode {
                    mode: ReadingPaneMode::RawText,
                },
                IntentEffect::Dispatch(Msg::ReadingPaneModeSet {
                    mode: ReadingPaneMode::RawText,
                }),
            ),
            (
                UiIntent::OpenArchiveDialog,
                IntentEffect::Dispatch(Msg::ArchiveClicked),
            ),
            (
                UiIntent::SubmitArchiveDialog {
                    request_id: 11,
                    basename: "archive".into(),
                    set_checkpoint: true,
                    use_summaries: true,
                    use_signal_candidates: false,
                },
                IntentEffect::Dispatch(Msg::ArchiveDialogSubmitted {
                    request_id: 11,
                    basename: "archive".into(),
                    set_checkpoint: true,
                    submitted_at: now,
                    use_summaries: true,
                    use_signal_candidates: false,
                }),
            ),
            (
                UiIntent::CancelArchiveDialog,
                IntentEffect::Host(HostAction::CancelArchiveDialog),
            ),
            (
                UiIntent::ToggleSignalCandidateExclusion {
                    signal_key: "signal".into(),
                },
                IntentEffect::Dispatch(Msg::ToggleSignalCandidateExclusion {
                    signal_key: "signal".into(),
                }),
            ),
            (
                UiIntent::SetUrlInput {
                    text: "https://example.com".into(),
                },
                IntentEffect::Dispatch(Msg::InputChanged("https://example.com".into())),
            ),
            (
                UiIntent::SubmitUrls,
                IntentEffect::Dispatch(Msg::UrlsSubmitted),
            ),
        ];

        for (intent, expected) in cases {
            assert_eq!(intent.into_effect(&context), expected);
        }
    }
}
