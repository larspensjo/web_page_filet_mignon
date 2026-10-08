use super::summary_cache_support::log_summary_cache_run_summary;
use crate::{AppState, Effect};
use engine_logging::{engine_info, engine_warn};
pub(super) fn handle_checkpoint_loaded(
    state: &mut AppState,
    since_utc: Option<String>,
) -> Vec<Effect> {
    let parsed = since_utc
        .as_deref()
        .and_then(|s| match chrono::DateTime::parse_from_rfc3339(s) {
            Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
            Err(e) => {
                engine_warn!(
                    "[briefing-checkpoint] file contained invalid RFC3339: {}",
                    e
                );
                None
            }
        });
    state.set_briefing_since_utc(parsed);
    state.clear_briefing_checkpoint_save_tracking();
    vec![]
}

pub(super) fn handle_checkpoint_save_succeeded(state: &mut AppState, save_id: u64) -> Vec<Effect> {
    if !state.finish_briefing_checkpoint_save_success(save_id) {
        return vec![];
    }
    engine_info!("[briefing-checkpoint] save succeeded save_id={}", save_id);
    state.mark_dirty();
    vec![]
}

pub(super) fn handle_checkpoint_save_failed(
    state: &mut AppState,
    save_id: u64,
    reason: String,
) -> Vec<Effect> {
    if !state.finish_briefing_checkpoint_save_failure(save_id, &reason) {
        return vec![];
    }
    engine_warn!(
        "[briefing-checkpoint] save failed save_id={} reason={}; reverted in-memory checkpoint",
        save_id,
        reason
    );
    state.mark_dirty();
    vec![]
}

pub(super) fn handle_checkpoint_set(state: &mut AppState, since: Option<String>) -> Vec<Effect> {
    let parsed = since
        .as_deref()
        .and_then(|s| match chrono::DateTime::parse_from_rfc3339(s) {
            Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
            Err(_) => {
                engine_warn!("[briefing-checkpoint] ignoring invalid timestamp: {}", s);
                None
            }
        });
    // If caller passed Some(bad string), treat as no-op
    if since.is_some() && parsed.is_none() {
        return vec![];
    }
    let save_id = state.begin_briefing_checkpoint_save(parsed);
    state.mark_dirty();
    vec![Effect::SaveBriefingCheckpoint {
        save_id,
        since_utc: parsed,
    }]
}

pub(super) fn settle_summaries(state: &mut AppState, effects: &mut Vec<Effect>) {
    if state.briefing().pending_count() > 0 || state.briefing().in_progress_count() > 0 {
        return;
    }
    if state.briefing().completed_summary_count() == 0 {
        state
            .briefing_mut()
            .fail("all article summaries failed".to_string());
    } else {
        state.briefing_mut().complete_without_briefing();
    }
    state.mark_dirty();
    log_summary_cache_run_summary(state);
    effects.push(Effect::FlushResults);
}
