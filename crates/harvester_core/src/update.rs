use crate::{
    calc_left_width, normalize_url_for_dedupe, AppState, Effect, Msg, SessionState, StopPolicy,
};

// Minimum width for the left panels (PANEL_INPUT + PANEL_JOBS)
const MIN_LEFT_WIDTH: i32 = 200;
// Minimum width for the preview panel
const MIN_PREVIEW_WIDTH: i32 = 200;
// Total width occupied by splitter (width + margins)
const SPLITTER_TOTAL_WIDTH: i32 = 16; // 4px bar + 6px margin each side

/// Pure update function: applies a message to state and returns any effects.
pub fn update(mut state: AppState, msg: Msg) -> (AppState, Vec<Effect>) {
    let effects = match msg {
        Msg::InputChanged(text) => {
            state.set_input_buffer(text);
            Vec::new()
        }
        Msg::UrlsSubmitted => {
            let raw = state.input_buffer().to_owned();
            // Phase 0 invariant: when paste handling grows, keep `SessionState::Finishing`
            // as a strict block (no auto-resume, no new intake) unless gated by a feature flag.
            let urls = parse_urls(&raw);
            if urls.is_empty() {
                return (state, Vec::new());
            }
            match state.session() {
                SessionState::Finishing | SessionState::Finished => {
                    return (state, Vec::new());
                }
                SessionState::Idle | SessionState::Running => {}
            }

            // Phase 4: deduplicate URLs before enqueuing
            let mut unique_urls = Vec::new();
            let mut skipped_count = 0;
            for url in urls {
                let normalized = normalize_url_for_dedupe(&url);
                if state.is_url_seen(&normalized) {
                    skipped_count += 1;
                } else {
                    unique_urls.push(url);
                }
            }

            // If all URLs were duplicates, we still update stats but don't enqueue or start
            if unique_urls.is_empty() {
                state.set_last_paste_stats(0, skipped_count);
                return (state, Vec::new());
            }

            let should_start = state.session() == SessionState::Idle;
            if should_start {
                state.start_session();
            }

            state.set_urls(unique_urls);
            let enqueued = state.enqueue_jobs_from_ui();
            let enqueued_count = enqueued.len();
            state.set_last_paste_stats(enqueued_count, skipped_count);
            if enqueued_count > 0 {
                state.clear_input_buffer();
            }
            let mut effects = Vec::with_capacity(enqueued.len() + usize::from(should_start));
            if should_start {
                effects.push(Effect::StartSession);
            }
            for (job_id, url) in enqueued {
                effects.push(Effect::EnqueueUrl { job_id, url });
            }
            effects
        }
        Msg::StopFinishClicked => {
            if state.session() == SessionState::Running {
                state.finish_session();
                vec![Effect::StopFinish {
                    policy: StopPolicy::Finish,
                }]
            } else {
                Vec::new()
            }
        }
        Msg::ArchiveClicked => vec![Effect::ArchiveRequested],
        Msg::JobProgress {
            job_id,
            stage,
            tokens,
            bytes,
            content_preview,
        } => {
            state.apply_progress(job_id, stage, tokens, bytes, content_preview);
            Vec::new()
        }
        Msg::JobDone {
            job_id,
            result,
            content_preview,
            extracted_links,
        } => {
            state.apply_done(job_id, result, content_preview, extracted_links);
            Vec::new()
        }
        Msg::LinkToggleRequested {
            job_id,
            link_index,
            checked,
        } => {
            let mut effects = Vec::new();
            if let Some((url, downloaded_path)) = state.link_metadata(job_id, link_index) {
                if checked && state.mark_link_download_requested(job_id, link_index) {
                    effects.push(Effect::DownloadLinkedPage {
                        job_id,
                        link_index,
                        url,
                    });
                } else if !checked && state.mark_link_deleted(job_id, link_index) {
                    if let Some(path) = downloaded_path {
                        effects.push(Effect::DeleteLinkedPage {
                            job_id,
                            link_index,
                            path,
                        });
                    }
                }
            }
            effects
        }
        Msg::LinkDownloadStarted { job_id, link_index } => {
            state.mark_link_download_requested(job_id, link_index);
            Vec::new()
        }
        Msg::LinkDownloadCompleted {
            job_id,
            link_index,
            path,
        } => {
            state.mark_link_download_completed(job_id, link_index, path);
            Vec::new()
        }
        Msg::LinkDownloadFailed {
            job_id,
            link_index,
            error,
        } => {
            state.mark_link_download_failed(job_id, link_index, error);
            Vec::new()
        }
        Msg::LinkDeleted { job_id, link_index } => {
            state.mark_link_deleted(job_id, link_index);
            Vec::new()
        }
        Msg::JobSelected { job_id } => {
            state.select_job(job_id);
            Vec::new()
        }
        Msg::RestoreCompletedJobs(entries) => {
            state.restore_completed_jobs(entries);
            Vec::new()
        }
        Msg::SplitterMoved {
            desired_left_width_px,
        } => {
            let clamped = calc_left_width(
                desired_left_width_px,
                state.window_width(),
                MIN_LEFT_WIDTH,
                MIN_PREVIEW_WIDTH,
                SPLITTER_TOTAL_WIDTH,
            );
            state.set_left_panel_width(clamped);
            state.mark_dirty();
            Vec::new()
        }
        Msg::WindowResized { window_width } => {
            state.set_window_width(window_width);
            // Re-clamp the left panel width based on new window width
            let clamped = calc_left_width(
                state.left_panel_width(),
                window_width,
                MIN_LEFT_WIDTH,
                MIN_PREVIEW_WIDTH,
                SPLITTER_TOTAL_WIDTH,
            );
            state.set_left_panel_width(clamped);
            state.mark_dirty();
            Vec::new()
        }
        Msg::Tick | Msg::NoOp => Vec::new(),
    };

    (state, effects)
}

fn parse_urls(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
