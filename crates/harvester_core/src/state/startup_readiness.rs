//! Reducer-owned startup outcomes, independent of run orchestration.
use super::AppState;
use crate::Msg;
use engine_logging::engine_warn;
use harvester_engine::llm::prompt::PromptId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartupInputOutcome {
    #[default]
    Pending,
    Loaded,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InitialArticleWindowOutcome {
    #[default]
    Pending,
    LoadedAndResolved,
    Empty,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupReadinessStatus {
    Pending,
    Ready,
    Unavailable,
}

#[derive(Debug, Clone, Copy)]
enum ResultStore {
    Summary,
    Triage,
    SignalCandidate,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct HydratedStores {
    summary: bool,
    triage: bool,
    signal_candidate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StartupReadiness {
    pub checkpoint: StartupInputOutcome,
    pub prompt_metadata: StartupInputOutcome,
    pub prompt_contexts: StartupInputOutcome,
    pub result_stores: StartupInputOutcome,
    hydrated_stores: HydratedStores,
    pub initial_article_window: InitialArticleWindowOutcome,
}
impl StartupReadiness {
    fn record_store_hydrated(&mut self, store: ResultStore) {
        match store {
            ResultStore::Summary => self.hydrated_stores.summary = true,
            ResultStore::Triage => self.hydrated_stores.triage = true,
            ResultStore::SignalCandidate => self.hydrated_stores.signal_candidate = true,
        }
        if self.result_stores != StartupInputOutcome::Failed
            && self.hydrated_stores.summary
            && self.hydrated_stores.triage
            && self.hydrated_stores.signal_candidate
        {
            self.result_stores = StartupInputOutcome::Loaded;
        }
    }
    fn inputs(&self) -> [StartupInputOutcome; 4] {
        [
            self.checkpoint,
            self.prompt_metadata,
            self.prompt_contexts,
            self.result_stores,
        ]
    }

    /// Restoration tolerates failed inputs, but needs resolved articles to select from.
    pub fn settled_with_articles(&self) -> bool {
        !self.inputs().contains(&StartupInputOutcome::Pending)
            && self.initial_article_window == InitialArticleWindowOutcome::LoadedAndResolved
    }

    pub fn status(&self) -> StartupReadinessStatus {
        use InitialArticleWindowOutcome as W;
        use StartupInputOutcome as I;
        let inputs = self.inputs();
        if inputs.contains(&I::Failed) || self.initial_article_window == W::Failed {
            StartupReadinessStatus::Unavailable
        } else if inputs.contains(&I::Pending) || self.initial_article_window == W::Pending {
            StartupReadinessStatus::Pending
        } else {
            StartupReadinessStatus::Ready
        }
    }
}
impl AppState {
    pub fn startup_inputs(&self) -> &StartupReadiness {
        &self.startup_inputs
    }
    pub fn startup_readiness(&self) -> StartupReadinessStatus {
        self.startup_inputs.status()
    }
    pub fn startup_settled_with_articles(&self) -> bool {
        self.startup_inputs.settled_with_articles()
    }
    // Startup outcomes still arrive if a run has already frozen its configuration.
    // Record the reply itself, independently of whether that run applies its payload.
    pub(crate) fn record_startup_reply(&mut self, msg: &Msg) {
        use StartupInputOutcome as I;
        match msg {
            Msg::BriefingCheckpointLoaded { .. } => self.startup_inputs.checkpoint = I::Loaded,
            Msg::PromptContextsLoaded { .. } => self.startup_inputs.prompt_contexts = I::Loaded,
            Msg::PromptContextsLoadFailed { .. } => self.startup_inputs.prompt_contexts = I::Failed,
            Msg::LlmMetadataLoaded {
                active_versions,
                effective_models,
            } => {
                let mut valid = true;
                for id in [
                    PromptId::ArticleTriage,
                    PromptId::ArticleSummary,
                    PromptId::ArticleSignalCandidate,
                ] {
                    let missing_version = !active_versions.contains_key(&id);
                    let missing_or_blank_model = effective_models
                        .get(&id)
                        .is_none_or(|model| model.trim().is_empty());
                    if missing_version || missing_or_blank_model {
                        valid = false;
                        engine_warn!(
                            "[startup-readiness] invalid prompt metadata prompt_id={} missing_version={} missing_or_blank_model={}",
                            id,
                            missing_version,
                            missing_or_blank_model
                        );
                    }
                }
                self.startup_inputs.prompt_metadata = if valid { I::Loaded } else { I::Failed };
            }
            Msg::SummaryCacheHydrated { .. } => self
                .startup_inputs
                .record_store_hydrated(ResultStore::Summary),
            Msg::TriageCacheHydrated { .. } => self
                .startup_inputs
                .record_store_hydrated(ResultStore::Triage),
            Msg::SignalCandidateCacheLoaded { .. } => self
                .startup_inputs
                .record_store_hydrated(ResultStore::SignalCandidate),
            Msg::ResultStoreUnavailable { .. }
                if self.startup_inputs.result_stores == I::Pending =>
            {
                // This message also reports later writes; only hydration sets readiness.
                self.startup_inputs.result_stores = I::Failed;
            }
            _ => {}
        }
    }
}
