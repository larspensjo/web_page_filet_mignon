use crate::{AppState, Effect, Msg, SessionState, StopPolicy};

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

            let should_start = state.session() == SessionState::Idle;
            if should_start {
                state.start_session();
            }

            state.set_urls(urls);
            let enqueued = state.enqueue_jobs_from_ui();
            let enqueued_count = enqueued.len();
            // Phase 4 will add deduplication; for now skipped is always 0
            state.set_last_paste_stats(enqueued_count, 0);
            let mut effects = Vec::with_capacity(enqueued.len() + usize::from(should_start));
            if should_start {
                effects.push(Effect::StartSession);
            }
            for (job_id, url) in enqueued {
                effects.push(Effect::EnqueueUrl { job_id, url });
            }
            effects
        }
        Msg::StartClicked => {
            if state.session() == SessionState::Idle {
                state.start_session();
                let enqueued = state.enqueue_jobs_from_ui();
                let mut effects = Vec::with_capacity(1 + enqueued.len());
                effects.push(Effect::StartSession);
                for (job_id, url) in enqueued {
                    effects.push(Effect::EnqueueUrl { job_id, url });
                }
                effects
            } else {
                Vec::new()
            }
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
        Msg::JobProgress {
            job_id,
            stage,
            tokens,
            bytes,
        } => {
            state.apply_progress(job_id, stage, tokens, bytes);
            Vec::new()
        }
        Msg::JobDone { job_id, result } => {
            state.apply_done(job_id, result);
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
