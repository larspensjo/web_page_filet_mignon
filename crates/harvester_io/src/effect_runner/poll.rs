use std::thread;
use std::time::Instant;

use engine_logging::{engine_info, engine_warn};
use harvester_core::Msg;
use harvester_engine::{poll_curated_source, poll_file_source, SourceKind, SourceType};

use crate::effect_helpers::{
    handle_brave_source_poll, handle_rss_source_poll, BravePollContext, PollGuard, RssPollContext,
};
use crate::{load_brave_seen_set, load_seen_set, load_sources};

use super::EffectRunner;

impl EffectRunner {
    pub(super) fn execute_poll_all_sources(&self) {
        let msg_tx = self.msg_tx.clone();
        let sources_path = self.paths.sources_path.clone();
        let seen_set_path = self.paths.seen_set_path.clone();
        let brave_seen_set_path = self.paths.brave_seen_set_path.clone();
        let brave_metadata_path = self.paths.brave_metadata_path.clone();
        let output_dir = self.paths.output_dir.clone();
        let url_policy = self.url_policy.clone();
        let fetch_settings = self.fetch_settings.clone();

        thread::spawn(move || {
            let _guard = PollGuard::new(msg_tx.clone());
            let poll_started = Instant::now();

            engine_info!("[source-config] loading {}", sources_path.display());
            let registry = load_sources(&sources_path);
            let config_dir = sources_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_path_buf();
            let allowed_dirs = vec![config_dir.clone(), output_dir.clone()];
            let enabled_sources: Vec<_> =
                registry.sources.into_iter().filter(|s| s.enabled).collect();

            let mut seen_set = load_seen_set(&seen_set_path);
            let mut brave_seen_set = load_brave_seen_set(&brave_seen_set_path);

            let _ = msg_tx.send(Msg::PollStarted {
                total: enabled_sources.len(),
            });

            for config in &enabled_sources {
                let config = config.clone();
                let source_id = config.id.clone();
                let source_started = Instant::now();
                match config.source_type {
                    SourceType::File { path } => match poll_file_source(
                        source_id.clone(),
                        &path,
                        &config_dir,
                        &allowed_dirs,
                        config.max_urls_per_poll,
                    ) {
                        Ok(result) => {
                            engine_info!(
                                "[file-poll] {} => {} URL(s)",
                                source_id,
                                result.urls.len()
                            );
                            engine_info!(
                                "[poll-all-timing] source={} kind=file status=ok urls={} elapsed_ms={}",
                                source_id,
                                result.urls.len(),
                                source_started.elapsed().as_millis()
                            );
                            let _ = msg_tx.send(Msg::SourcePollCompleted {
                                source_id,
                                urls: result.urls,
                                kind: SourceKind::File,
                                parsed: result.parsed,
                                dedup_filtered: result.dedup_filtered,
                            });
                        }
                        Err(err) => {
                            engine_warn!("[file-poll] {} failed: {}", source_id, err);
                            engine_warn!(
                                "[poll-all-timing] source={} kind=file status=err elapsed_ms={} error={}",
                                source_id,
                                source_started.elapsed().as_millis(),
                                err
                            );
                            let _ = msg_tx.send(Msg::SourcePollFailed {
                                source_id,
                                error: err.to_string(),
                            });
                        }
                    },
                    SourceType::CuratedList { urls } => {
                        let result =
                            poll_curated_source(source_id.clone(), &urls, config.max_urls_per_poll);
                        engine_info!(
                            "[curated-poll] {} => {} URL(s)",
                            source_id,
                            result.urls.len()
                        );
                        engine_info!(
                            "[poll-all-timing] source={} kind=curated status=ok urls={} elapsed_ms={}",
                            source_id,
                            result.urls.len(),
                            source_started.elapsed().as_millis()
                        );
                        let _ = msg_tx.send(Msg::SourcePollCompleted {
                            source_id,
                            urls: result.urls,
                            kind: SourceKind::Curated,
                            parsed: result.parsed,
                            dedup_filtered: result.dedup_filtered,
                        });
                    }
                    SourceType::Script { .. } => {
                        engine_warn!("[poll-all] Script sources not yet supported: {}", source_id);
                        engine_warn!(
                            "[poll-all-timing] source={} kind=script status=unsupported elapsed_ms={}",
                            source_id,
                            source_started.elapsed().as_millis()
                        );
                        let _ = msg_tx.send(Msg::SourcePollFailed {
                            source_id,
                            error: "Script sources not implemented".to_string(),
                        });
                    }
                    SourceType::Rss { feed_url } => {
                        let mut context = RssPollContext {
                            seen_set: &mut seen_set,
                            seen_set_path: &seen_set_path,
                            msg_tx: &msg_tx,
                        };
                        handle_rss_source_poll(
                            &source_id,
                            &feed_url,
                            &url_policy,
                            &fetch_settings,
                            &mut context,
                            config.max_urls_per_poll,
                        );
                        engine_info!(
                            "[poll-all-timing] source={} kind=rss elapsed_ms={}",
                            source_id,
                            source_started.elapsed().as_millis()
                        );
                    }
                    SourceType::BraveNews(ref cfg) => {
                        let mut context = BravePollContext {
                            brave_seen_set: &mut brave_seen_set,
                            brave_seen_set_path: &brave_seen_set_path,
                            brave_metadata_path: &brave_metadata_path,
                            msg_tx: &msg_tx,
                        };
                        handle_brave_source_poll(
                            &source_id,
                            cfg,
                            config.max_urls_per_poll,
                            &fetch_settings,
                            &mut context,
                        );
                        engine_info!(
                            "[poll-all-timing] source={} kind=brave elapsed_ms={}",
                            source_id,
                            source_started.elapsed().as_millis()
                        );
                    }
                }
            }
            engine_info!(
                "[poll-all-timing] all-sources completed enabled_sources={} elapsed_ms={}",
                enabled_sources.len(),
                poll_started.elapsed().as_millis()
            );
        });
    }
}
