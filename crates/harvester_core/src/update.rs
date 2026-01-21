use crate::{AppState, Effect, Msg, SessionState, StopPolicy};

/// Pure update function: applies a message to state and returns any effects.
pub fn update(mut state: AppState, msg: Msg) -> (AppState, Vec<Effect>) {
    let effects = match msg {
        Msg::UrlsPasted(raw) => {
            let urls = parse_urls(&raw);
            state.set_urls(urls);
            Vec::new()
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
