use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

use chrono::Utc;
use commanductui::types::MessageSeverity;
use commanductui::{AppEvent, ControlId, PlatformCommand, PlatformEventHandler, WindowId};
use engine_logging::{engine_info, engine_warn};
use harvester_core::{
    update, AppTab, AppViewModel, Effect, JobListScope, JobResultKind, LayoutViewModel, LeftTab,
    Msg, PromptLabStage, SignalCandidateState, TrendCategory,
};
use harvester_io::PersistenceSnapshot;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN};

use super::archive_dialog::{
    archive_field_checked, archive_field_text, build_archive_form_descriptor,
    parse_archive_dialog_request_id, ARCHIVE_DIALOG_FILENAME_FIELD_ID,
    ARCHIVE_DIALOG_SET_CHECKPOINT_FIELD_ID, ARCHIVE_DIALOG_USE_SIGNAL_CANDIDATES_FIELD_ID,
    ARCHIVE_DIALOG_USE_SUMMARIES_FIELD_ID,
};
use super::pre_triage_toggle_message;
use super::render_batch::{
    is_geometry_only_message, select_render_mode, GeometryBatchStats, PendingRender, RenderMode,
};
use super::ui;
use super::{AppEventHandler, PendingFocus};

pub(super) const VK_RETURN_CODE: u16 = VK_RETURN.0;
pub(super) const VK_ESCAPE_CODE: u16 = VK_ESCAPE.0;

impl AppEventHandler {
    pub(super) fn new(
        window_id: WindowId,
        shared: Arc<Mutex<super::SharedState>>,
        msg_rx: mpsc::Receiver<Msg>,
        msg_tx: mpsc::Sender<Msg>,
        effect_runner: super::EffectRunner,
        persistence_worker: super::PersistenceWorker,
        tree_render_state: ui::render::TreeRenderState,
    ) -> Self {
        Self {
            window_id,
            shared,
            commands: VecDeque::new(),
            msg_rx: Mutex::new(msg_rx),
            msg_tx,
            effect_runner,
            persistence_worker,
            tree_render_state,
            pending_focus_after_render: Vec::new(),
        }
    }

    pub(super) fn process_pending_messages(&mut self) {
        let mut inbox = Vec::new();
        if let Ok(rx) = self.msg_rx.lock() {
            while let Ok(msg) = rx.try_recv() {
                inbox.push(msg);
            }
        }
        if inbox.is_empty() {
            return;
        }
        let batch_size = inbox.len();
        let mut geometry_stats = GeometryBatchStats::default();
        for msg in &inbox {
            geometry_stats.observe(msg);
        }
        let geometry_only_batch = inbox.iter().all(is_geometry_only_message);

        let mut clear_input_needed = false;
        let mut persist_completed_needed = false;
        let mut persist_overrides_needed = false;
        let mut archive_failure_notice: Option<(String, String)> = None;
        let mut refresh_evaluation_dispatched = false;
        let mut persistence_enqueued = false;
        let mut queued_effects = Vec::new();
        let (maybe_render, render_mode, render_snapshot_ms, rendered_job_count) = {
            let mut guard = self.shared.lock().expect("lock shared state");
            let mut state = std::mem::take(&mut guard.state);
            let mut any_dirty = false;

            for msg in inbox {
                let msg_for_flags = msg.clone();
                let (next_state, effects) = update(state, msg);
                state = next_state;
                persist_completed_needed |= matches!(
                    msg_for_flags,
                    Msg::JobDone {
                        result: JobResultKind::Success,
                        ..
                    }
                );
                persist_overrides_needed |= matches!(
                    msg_for_flags,
                    Msg::PreTriageDecisionSet { .. } | Msg::PreTriageResetClicked
                );
                if let Msg::ArchiveExportFailed {
                    basename, reason, ..
                } = msg_for_flags
                {
                    archive_failure_notice =
                        Some((format!("Archive export failed: {basename}"), reason));
                }
                clear_input_needed |= effects
                    .iter()
                    .any(|effect| matches!(effect, Effect::EnqueueUrl { .. }));
                queued_effects.extend(effects);
                any_dirty |= state.consume_dirty();
            }

            if let Some(triggered_by_job_done) = state.take_pre_triage_refresh_evaluation_request()
            {
                refresh_evaluation_dispatched = true;
                let ordered_urls = state.ordered_completed_job_urls_snapshot();
                let (next_state, effects) = update(
                    state,
                    Msg::EvaluatePreTriageRefresh {
                        ordered_urls,
                        triggered_by_job_done,
                    },
                );
                state = next_state;
                queued_effects.extend(effects);
                any_dirty |= state.consume_dirty();
            }

            let persistence_snapshot = if persist_completed_needed || persist_overrides_needed {
                Some(PersistenceSnapshot::capture(&state))
            } else {
                None
            };
            let render_mode = select_render_mode(
                any_dirty,
                geometry_only_batch,
                refresh_evaluation_dispatched,
                clear_input_needed,
                queued_effects.len(),
            );
            let snapshot_start = Instant::now();
            let maybe_render = match render_mode {
                Some(RenderMode::Full) => {
                    let view = state.view();
                    let job_count = view.job_count;
                    (
                        Some(PendingRender::Full(Box::new(view))),
                        Some(RenderMode::Full),
                        job_count,
                    )
                }
                Some(RenderMode::LayoutOnly) => (
                    Some(PendingRender::LayoutOnly(state.layout_view())),
                    Some(RenderMode::LayoutOnly),
                    0,
                ),
                None => (None, None, 0),
            };
            let render_snapshot_ms = snapshot_start.elapsed().as_millis();
            guard.state = state;

            if let Some(snapshot) = persistence_snapshot {
                persistence_enqueued = true;
                self.persistence_worker.enqueue(snapshot);
            }
            (
                maybe_render.0,
                maybe_render.1,
                render_snapshot_ms,
                maybe_render.2,
            )
        };

        let mut effect_runner_effects = Vec::new();
        for effect in queued_effects {
            match effect {
                Effect::ShowArchiveDialog {
                    request_id,
                    article_count,
                    since_utc,
                    default_basename,
                    default_file_exists,
                    export_dir,
                    pending_pre_triage_count,
                    token_estimates,
                    signal_candidate_default,
                    signal_candidate_count,
                    signal_candidate_scoring_done,
                    signal_candidate_scoring_total,
                    signal_candidate_token_estimates,
                } => {
                    let form = build_archive_form_descriptor(
                        request_id,
                        article_count,
                        since_utc,
                        default_basename,
                        default_file_exists,
                        export_dir,
                        pending_pre_triage_count,
                        token_estimates,
                        signal_candidate_default,
                        signal_candidate_count,
                        signal_candidate_scoring_done,
                        signal_candidate_scoring_total,
                        signal_candidate_token_estimates,
                    );
                    self.commands.push_back(PlatformCommand::ShowFormDialog {
                        window_id: self.window_id,
                        form,
                    });
                }
                other => effect_runner_effects.push(other),
            }
        }

        let effect_count = effect_runner_effects.len();
        self.effect_runner.enqueue(effect_runner_effects);
        let did_work = effect_count > 0
            || maybe_render.is_some()
            || clear_input_needed
            || persistence_enqueued
            || refresh_evaluation_dispatched;
        if did_work {
            engine_info!("[msg-loop] batch_size={} queue_lag_ms={}", batch_size, 0);
        }

        if clear_input_needed {
            self.commands.push_back(PlatformCommand::SetInputText {
                window_id: self.window_id,
                control_id: ui::constants::INPUT_URLS,
                text: String::new(),
            });
        }

        if let Some((title, message)) = archive_failure_notice {
            self.commands.push_back(PlatformCommand::ShowMessageBox {
                window_id: self.window_id,
                title,
                message,
                severity: MessageSeverity::Error,
            });
        }

        let mut render_enqueue_ms = 0;
        match maybe_render {
            Some(PendingRender::Full(view)) => {
                let render_start = Instant::now();
                self.enqueue_render(&view);
                render_enqueue_ms = render_start.elapsed().as_millis();
            }
            Some(PendingRender::LayoutOnly(layout)) => {
                let render_start = Instant::now();
                self.enqueue_layout_render(&layout);
                render_enqueue_ms = render_start.elapsed().as_millis();
            }
            None => {}
        }
        self.enqueue_pending_focus_commands();

        if !geometry_stats.is_empty() {
            engine_info!(
                "[ui-geometry] splitter_moves={} window_resizes={} last_splitter_width={:?} last_window_width={:?} render_mode={:?} snapshot_ms={} enqueue_ms={} job_count={}",
                geometry_stats.splitter_moves,
                geometry_stats.window_resizes,
                geometry_stats.last_splitter_width,
                geometry_stats.last_window_width,
                render_mode,
                render_snapshot_ms,
                render_enqueue_ms,
                rendered_job_count,
            );
        }
    }

    pub(super) fn enqueue_render(&mut self, view: &AppViewModel) {
        self.commands.extend(ui::render::render(
            self.window_id,
            view,
            &mut self.tree_render_state,
        ));
    }

    pub(super) fn enqueue_layout_render(&mut self, layout: &LayoutViewModel) {
        self.commands.extend(ui::render::render_layout_only(
            self.window_id,
            layout,
            &mut self.tree_render_state,
        ));
    }

    pub(super) fn queue_focus_after_render(&mut self, control_id: ControlId, select_all: bool) {
        self.pending_focus_after_render.push(PendingFocus {
            control_id,
            select_all,
        });
    }

    pub(super) fn enqueue_pending_focus_commands(&mut self) {
        let pending = std::mem::take(&mut self.pending_focus_after_render);
        for PendingFocus {
            control_id,
            select_all,
        } in pending
        {
            self.commands.push_back(PlatformCommand::SetFocus {
                window_id: self.window_id,
                control_id,
                select_all,
            });
        }
    }
}

impl Drop for AppEventHandler {
    fn drop(&mut self) {
        self.persistence_worker.shutdown();
    }
}

fn msg_for_preview_context_button(control_id: ControlId) -> Option<Msg> {
    match control_id {
        ui::constants::BUTTON_PREVIEW_SOURCE_LINK => Some(Msg::OpenInBrowserClicked),
        _ => None,
    }
}

impl PlatformEventHandler for AppEventHandler {
    fn handle_event(&mut self, event: AppEvent) {
        // Invariant: every handler observes state with all prior AppEvents
        // already applied. Without this, a handler that reads `self.shared`
        // to decide what `Msg` to send (e.g. the Jobs-search Enter branch)
        // sees state one keystroke behind the widget. Cheap when the channel
        // is empty — `process_pending_messages` short-circuits on no msgs.
        self.process_pending_messages();
        if let AppEvent::ButtonClicked { control_id, .. } = &event {
            if let Some(msg) = msg_for_preview_context_button(*control_id) {
                let _ = self.msg_tx.send(msg);
                return;
            }
            if let Some(msg) = ui::groups::bottom_buttons::msg_for_control(*control_id) {
                let _ = self.msg_tx.send(msg);
                return;
            }
            if let Some(msg) = ui::groups::prompt_lab_actions::msg_for_button(*control_id) {
                let _ = self.msg_tx.send(msg);
                return;
            }
        }
        if let AppEvent::CheckBoxToggled { control_id, .. } = &event {
            if let Some(msg) = ui::groups::prompt_lab_actions::msg_for_checkbox(*control_id) {
                let _ = self.msg_tx.send(msg);
                return;
            }
            if let Some(msg) = ui::groups::prompt_lab_sections::msg_for_checkbox(*control_id) {
                let _ = self.msg_tx.send(msg);
                return;
            }
            if *control_id == ui::constants::CHK_SIGNAL_CANDIDATE_EXCLUDE {
                let guard = self.shared.lock().unwrap();
                if let Some(job_id) = guard.state.selected_job_id() {
                    if let Some(url) = guard.state.job_url_for(job_id) {
                        if let Some(SignalCandidateState::Completed { result }) =
                            guard.state.signal_candidate().state_for(url)
                        {
                            let signal_key = result.signal_key.clone();
                            drop(guard);
                            let _ = self
                                .msg_tx
                                .send(Msg::ToggleSignalCandidateExclusion { signal_key });
                        }
                    }
                }
                return;
            }
        }

        match event {
            AppEvent::MainWindowUISetupComplete { .. } => {
                let _ = self.msg_tx.send(Msg::Tick);
            }
            AppEvent::TabBarSelectionChanged {
                control_id,
                selected_index,
                ..
            } if control_id == ui::constants::TAB_BAR_RIGHT => {
                if let Some(tab) = AppTab::from_index(selected_index) {
                    let _ = self.msg_tx.send(Msg::TabSelected { tab });
                }
            }
            AppEvent::TabBarSelectionChanged {
                control_id,
                selected_index,
                ..
            } if control_id == ui::constants::TAB_BAR_LEFT => {
                if let Some(tab) = LeftTab::from_index(selected_index) {
                    let _ = self.msg_tx.send(Msg::LeftTabSelected { tab });
                }
            }
            AppEvent::TabBarSelectionChanged {
                control_id,
                selected_index,
                ..
            } if control_id == ui::constants::TAB_BAR_TRENDS => {
                if let Some(category) = TrendCategory::from_index(selected_index) {
                    let _ = self.msg_tx.send(Msg::TrendCategorySelected { category });
                }
            }
            AppEvent::RadioButtonSelected { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_MODE_BASIC =>
            {
                let _ = self
                    .msg_tx
                    .send(Msg::PromptLabAdvancedModeSet { enabled: false });
            }
            AppEvent::RadioButtonSelected { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_MODE_ADVANCED =>
            {
                let _ = self
                    .msg_tx
                    .send(Msg::PromptLabAdvancedModeSet { enabled: true });
            }
            AppEvent::ToggleSwitchToggled {
                control_id,
                checked,
                ..
            } if control_id == ui::constants::TS_JOBS_SCOPE => {
                let scope = if checked {
                    JobListScope::SinceCheckpoint
                } else {
                    JobListScope::All
                };
                let _ = self.msg_tx.send(Msg::JobListScopeSet { scope });
            }
            AppEvent::RadioButtonSelected { control_id, .. }
                if control_id == ui::constants::BTN_STAGE_TRIAGE =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabStageSelected {
                    stage: PromptLabStage::Triage,
                });
            }
            AppEvent::RadioButtonSelected { control_id, .. }
                if control_id == ui::constants::BTN_STAGE_SUMMARY =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabStageSelected {
                    stage: PromptLabStage::Summary,
                });
            }
            AppEvent::RadioButtonSelected { control_id, .. }
                if control_id == ui::constants::BTN_STAGE_BRIEFING =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabStageSelected {
                    stage: PromptLabStage::Briefing,
                });
            }
            AppEvent::ComboBoxSelectionChanged {
                control_id,
                selected_index: Some(index),
                window_id: _,
            } if control_id == ui::constants::COMBO_PROMPT_LAB_MODEL_SELECTOR => {
                let guard = self.shared.lock().unwrap();
                let view = guard.state.view();
                let model = ui::render::combo_index_to_model(
                    index,
                    &view.left_pane.prompt_lab.model_catalog,
                );
                let _ = self.msg_tx.send(Msg::PromptLabModelOverrideSet { model });
            }
            AppEvent::MenuActionClicked { action_id }
                if action_id == ui::constants::MENU_ACTION_ADD_URL =>
            {
                let _ = self.msg_tx.send(Msg::ToggleInputPanel);
            }
            AppEvent::MenuActionClicked { action_id }
                if action_id == ui::constants::MENU_ACTION_FIND_JOBS =>
            {
                let _ = self.msg_tx.send(Msg::FocusJobsSearchRequested);
                self.queue_focus_after_render(ui::constants::INPUT_JOBS_SEARCH, true);
            }
            AppEvent::FormDialogCompleted {
                window_id,
                context_tag,
                confirmed,
                field_values,
            } if window_id == self.window_id => {
                if !confirmed {
                    return;
                }
                let Some(request_id) = parse_archive_dialog_request_id(&context_tag) else {
                    engine_warn!(
                        "[archive-dialog] ignoring form dialog result with unrecognized context_tag={}",
                        context_tag
                    );
                    return;
                };
                let basename = archive_field_text(&field_values, ARCHIVE_DIALOG_FILENAME_FIELD_ID)
                    .unwrap_or_default();
                let set_checkpoint =
                    archive_field_checked(&field_values, ARCHIVE_DIALOG_SET_CHECKPOINT_FIELD_ID)
                        .unwrap_or(true);
                let use_summaries =
                    archive_field_checked(&field_values, ARCHIVE_DIALOG_USE_SUMMARIES_FIELD_ID)
                        .unwrap_or(true);
                let use_signal_candidates = archive_field_checked(
                    &field_values,
                    ARCHIVE_DIALOG_USE_SIGNAL_CANDIDATES_FIELD_ID,
                )
                .unwrap_or(false);
                engine_info!(
                    "[archive-dialog] submitted request_id={} basename={} set_checkpoint={} use_summaries={} use_signal_candidates={}",
                    request_id,
                    basename,
                    set_checkpoint,
                    use_summaries,
                    use_signal_candidates
                );
                let _ = self.msg_tx.send(Msg::ArchiveDialogSubmitted {
                    request_id,
                    basename,
                    set_checkpoint,
                    submitted_at: Utc::now(),
                    use_summaries,
                    use_signal_candidates,
                });
            }
            AppEvent::InputTextChanged {
                control_id, text, ..
            } if control_id == ui::constants::INPUT_URLS => {
                engine_info!(
                    "InputTextChanged: {} chars, preview=\"{}\"",
                    text.len(),
                    text.chars().take(120).collect::<String>()
                );
                let _ = self.msg_tx.send(Msg::InputChanged(text));
                let _ = self.msg_tx.send(Msg::UrlsSubmitted);
            }
            AppEvent::InputTextChanged {
                control_id, text, ..
            } if control_id == ui::constants::INPUT_PROMPT_LAB_URL => {
                let _ = self
                    .msg_tx
                    .send(Msg::PromptLabUrlInputChanged { url: text });
            }
            AppEvent::InputTextChanged {
                control_id, text, ..
            } if control_id == ui::constants::INPUT_PROMPT_LAB_CONTEXT => {
                let _ = self.msg_tx.send(Msg::PromptLabContextDraftChanged { text });
            }
            AppEvent::InputTextChanged {
                control_id, text, ..
            } if control_id == ui::constants::INPUT_PROMPT_LAB_TEMPLATE_SYSTEM => {
                let _ = self
                    .msg_tx
                    .send(Msg::PromptLabTemplateSystemDraftChanged { text });
            }
            AppEvent::InputTextChanged {
                control_id, text, ..
            } if control_id == ui::constants::INPUT_PROMPT_LAB_TEMPLATE_USER => {
                let _ = self
                    .msg_tx
                    .send(Msg::PromptLabTemplateUserDraftChanged { text });
            }
            AppEvent::InputTextChanged {
                control_id, text, ..
            } if control_id == ui::constants::INPUT_JOBS_SEARCH => {
                self.tree_render_state
                    .note_jobs_search_input_text_from_user(text.clone());
                let _ = self.msg_tx.send(Msg::JobsSearchQueryChanged(text));
            }
            AppEvent::InputKeyDown {
                window_id,
                control_id,
                key_code,
                ..
            } if window_id == self.window_id && control_id == ui::constants::INPUT_JOBS_SEARCH => {
                match key_code {
                    VK_ESCAPE_CODE => {
                        let _ = self.msg_tx.send(Msg::JobsSearchCleared);
                        self.queue_focus_after_render(ui::constants::TREE_JOBS, false);
                    }
                    VK_RETURN_CODE => {
                        let first_visible = {
                            let guard = self.shared.lock().unwrap();
                            let view = guard.state.view();
                            if view.left_pane.selected_jobs_visible_in_filter {
                                None
                            } else {
                                view.left_pane.first_visible_job_id
                            }
                        };
                        if let Some(job_id) = first_visible {
                            let _ = self.msg_tx.send(Msg::JobSelected { job_id });
                            self.queue_focus_after_render(ui::constants::TREE_JOBS, false);
                        } else {
                            self.queue_focus_after_render(ui::constants::TREE_JOBS, false);
                        }
                    }
                    _ => {}
                }
            }
            AppEvent::ListBoxItemSelectionChanged {
                window_id, item_id, ..
            } if window_id == self.window_id => {
                let _ = self.msg_tx.send(Msg::JobSelected {
                    job_id: item_id.raw(),
                });
            }
            AppEvent::ListBoxItemKeyDown {
                window_id,
                control_id,
                key_code,
            } if window_id == self.window_id
                && control_id == ui::constants::TREE_JOBS
                && key_code == b'X' as u16 =>
            {
                let maybe_msg = {
                    let guard = self.shared.lock().unwrap();
                    if guard.state.left_tab() == LeftTab::TriageReview
                        && guard.state.is_pre_triage_reviewing()
                    {
                        pre_triage_toggle_message(&guard.state)
                    } else {
                        None
                    }
                };
                if let Some(msg) = maybe_msg {
                    let _ = self.msg_tx.send(msg);
                }
            }
            AppEvent::ListBoxScrolled { .. } => {}
            AppEvent::WindowCloseRequestedByUser { .. } => {
                self.commands.push_back(PlatformCommand::QuitApplication);
            }
            AppEvent::SplitterDragging {
                desired_left_width_px,
                ..
            } => {
                // Continuous dragging - update layout in real-time
                let _ = self.msg_tx.send(Msg::SplitterMoved {
                    desired_left_width_px,
                });
            }
            AppEvent::SplitterDragEnded {
                desired_left_width_px,
                ..
            } => {
                // Log boundary event: drag completed
                engine_info!(
                    "Splitter drag ended: left_panel_width={}",
                    desired_left_width_px
                );
                let _ = self.msg_tx.send(Msg::SplitterMoved {
                    desired_left_width_px,
                });
            }
            AppEvent::WindowResized {
                window_id, width, ..
            } if window_id == self.window_id => {
                let _ = self.msg_tx.send(Msg::WindowResized {
                    window_width: width,
                });
            }
            AppEvent::WindowResizeCompleted {
                window_id,
                outer_width,
                outer_height,
            } if window_id == self.window_id => {
                let _ = self.msg_tx.send(Msg::WindowResizeCompleted {
                    outer_width,
                    outer_height,
                });
            }
            _ => {}
        }
    }

    fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
        self.process_pending_messages();
        self.commands.pop_front()
    }
}
