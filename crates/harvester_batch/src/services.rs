use crate::{no_progress::NO_PROGRESS_TIMEOUT, Args};
use harvester_core::{Effect, Msg};
use harvester_io::{
    credentials::{read_host_keys, StoredKeyAccess, StoredKeyReads},
    host_bootstrap::{
        build_effect_runner, read_startup_key_outcomes, EffectRunnerBuild, HostKeys,
        HostLlmDefaults,
    },
    EffectRunner, NoOpPlatformHandler, PersistenceWorker, RuntimePaths,
};
use std::{sync::mpsc, time::Duration};

pub type InterruptHandler = Box<dyn Fn() + Send + Sync>;
pub(crate) type RunnerBuilder = dyn FnOnce(
    &RuntimePaths,
    &Args,
    mpsc::Sender<Msg>,
    HostKeys,
    &HostLlmDefaults,
) -> Result<EffectRunnerBuild, String>;
type EffectRouter = dyn Fn(&EffectRunner, Vec<Effect>);

/// Batch host dependencies at the boundaries for credentials, effects, and signals.
pub struct HostServices {
    pub(crate) read_keys: Box<dyn FnOnce() -> StoredKeyReads>,
    pub(crate) build_runner: Box<RunnerBuilder>,
    pub(crate) install_interrupt: Box<dyn FnOnce(InterruptHandler)>,
    pub(crate) route_effects: Box<EffectRouter>,
    pub(crate) watchdog_timeout: Duration,
}

impl HostServices {
    pub fn production(use_stored_keys: bool) -> Self {
        Self::with_stored_access(StoredKeyAccess::from_launch_flag(use_stored_keys))
    }

    pub(crate) fn with_stored_access(access: Option<StoredKeyAccess>) -> Self {
        Self::with_adapters(
            move || read_startup_key_outcomes(access.as_ref(), read_host_keys),
            |paths, args, tx, keys, defaults| {
                build_effect_runner(
                    paths,
                    tx,
                    args.llm_concurrency,
                    defaults,
                    Box::new(NoOpPlatformHandler),
                    Box::new(PersistenceWorker::new(
                        paths.state_path.clone(),
                        paths.blacklist_path.clone(),
                    )),
                    keys,
                    crate::runner::BATCH_AI_DISABLED_WARNING,
                )
            },
            |handler| ctrlc::set_handler(handler).expect("Error setting signal handler"),
        )
    }

    pub fn with_adapters(
        read_keys: impl FnOnce() -> StoredKeyReads + 'static,
        build_runner: impl FnOnce(
                &RuntimePaths,
                &Args,
                mpsc::Sender<Msg>,
                HostKeys,
                &HostLlmDefaults,
            ) -> Result<EffectRunnerBuild, String>
            + 'static,
        install_interrupt: impl FnOnce(InterruptHandler) + 'static,
    ) -> Self {
        Self {
            read_keys: Box::new(read_keys),
            build_runner: Box::new(build_runner),
            install_interrupt: Box::new(install_interrupt),
            route_effects: Box::new(EffectRunner::enqueue),
            watchdog_timeout: NO_PROGRESS_TIMEOUT,
        }
    }

    pub fn with_watchdog_timeout(mut self, timeout: Duration) -> Self {
        self.watchdog_timeout = timeout;
        self
    }

    pub fn with_effect_router(
        mut self,
        route_effects: impl Fn(&EffectRunner, Vec<Effect>) + 'static,
    ) -> Self {
        self.route_effects = Box::new(route_effects);
        self
    }
}
