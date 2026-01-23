use crate::{normalize_url_for_dedupe, AppState, Effect, Msg, SessionState, StopPolicy};

/// Pure update function: applies a message to state and returns any effects.
pub fn update(mut state: AppState, msg: Msg) -> (AppState, Vec<Effect>) {
    let effects = match msg {
        Msg::UrlsPasted(raw) => {
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
        } => {
            state.apply_progress(job_id, stage, tokens, bytes);
            Vec::new()
        }
        Msg::JobDone {
            job_id,
            result,
            content_preview,
        } => {
            state.apply_done(job_id, result, content_preview);
            Vec::new()
        }
        Msg::JobSelected { job_id } => {
            state.select_job(job_id);
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
