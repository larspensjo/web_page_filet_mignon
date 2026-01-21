use crate::{AppState, Effect, Msg, SessionState};

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
                state.enqueue_jobs_from_ui();
            }
            Vec::new()
        }
        Msg::StopFinishClicked => {
            if state.session() == SessionState::Running {
                state.finish_session();
            }
            Vec::new()
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
