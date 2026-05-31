use super::ui::tree_item_ids::{job_tree_item_id, link_tree_item_id};
use super::*;
use commanductui::types::{TreeItemMarkerKind, WindowId};
use commanductui::AppEvent;
use harvester_core::{
    AppState, CompletedJobSnapshot, Effect, JobResultKind, LeftTab, LlmResultKind, LoadedArticle,
    Msg,
};
use harvester_engine::llm::prompt::PromptVersion;
use harvester_engine::{ExtractedLink, LinkKind};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};

fn completed_job_snapshot(url: &str) -> CompletedJobSnapshot {
    CompletedJobSnapshot {
        url: url.to_string(),
        tokens: Some(123),
        bytes: Some(456),
        links: vec![],
        fetched_utc: None,
    }
}

fn loaded_article(url: &str) -> LoadedArticle {
    LoadedArticle {
        url: url.to_string(),
        source_title: Some("Example".to_string()),
        prepared_text: std::iter::repeat_n("triage-content", 220)
            .collect::<Vec<_>>()
            .join(" "),
        content_hash: "triage-hash".to_string(),
        fetched_utc: None,
    }
}

fn triage_json(priority: u8) -> String {
    format!(
        r#"{{"category":"security","priority":{},"tags":["tag"],"rationale":"reason"}}"#,
        priority
    )
}

fn triage_success_result(priority: u8) -> LlmResultKind {
    let prompt_version: PromptVersion = 1;
    LlmResultKind::Success {
        output_json: triage_json(priority),
        input_tokens: 42,
        output_tokens: 7,
        prompt_version,
        resolved_model: "test-model".to_string(),
    }
}

fn extract_load_articles_request_id(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("expected LoadArticlesForTriage effect")
}

fn extract_llm_request_id(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::RequestLlmCompletion { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("expected RequestLlmCompletion effect")
}

fn advance_to_triage_article_load(mut state: AppState) -> (AppState, u64) {
    for _ in 0..8 {
        let (next_state, effects) = update(state, Msg::Tick);
        state = next_state;
        if effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadArticlesForTriage { .. }))
        {
            return (state, extract_load_articles_request_id(&effects));
        }
    }
    panic!("expected LoadArticlesForTriage effect after ticking");
}

fn shared_state_with_triage_priority(priority: u8) -> Arc<Mutex<SharedState>> {
    let url = "https://example.com/triage";
    let (mut state, _) = update(
        AppState::new(),
        Msg::RestoreCompletedJobs(vec![completed_job_snapshot(url)]),
    );
    if let Some(triggered_by_job_done) = state.take_pre_triage_refresh_evaluation_request() {
        let ordered_urls = state.ordered_completed_job_urls_snapshot();
        let (next_state, _) = update(
            state,
            Msg::EvaluatePreTriageRefresh {
                ordered_urls,
                triggered_by_job_done,
            },
        );
        state = next_state;
    }
    let (state, load_request_id) = advance_to_triage_article_load(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_request_id,
            articles: vec![loaded_article(url)],
        },
    );
    let (state, _) = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: {
                let mut versions = std::collections::HashMap::new();
                versions.insert(PromptId::ArticleTriage, 1);
                versions
            },
            effective_models: {
                let mut models = std::collections::HashMap::new();
                models.insert(PromptId::ArticleTriage, "test-model".to_string());
                models
            },
            templates: std::collections::HashMap::<
                PromptId,
                harvester_core::PromptLabTemplateSnapshot,
            >::new(),
        },
    );
    let (state, _) = update(
        state,
        Msg::PromptContextsLoaded {
            contexts: std::collections::HashMap::new(),
        },
    );
    let (state, effects) = update(state, Msg::TriageClicked);
    let triage_request_id = extract_llm_request_id(&effects);
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: triage_request_id,
            result: triage_success_result(priority),
            metadata: None,
        },
    );
    let mut state = state;
    if let Some(triggered_by_job_done) = state.take_pre_triage_refresh_evaluation_request() {
        let ordered_urls = vec![url.to_string()];
        let (next_state, _) = update(
            state,
            Msg::EvaluatePreTriageRefresh {
                ordered_urls,
                triggered_by_job_done,
            },
        );
        state = next_state;
    }
    Arc::new(Mutex::new(SharedState { state }))
}

fn shared_state_with_ready_pre_triage_review() -> Arc<Mutex<SharedState>> {
    let url = "https://example.com/pre-triage";
    let (mut state, _) = update(
        AppState::new(),
        Msg::RestoreCompletedJobs(vec![completed_job_snapshot(url)]),
    );
    if let Some(triggered_by_job_done) = state.take_pre_triage_refresh_evaluation_request() {
        let (mut state, _) = update(
            state,
            Msg::EvaluatePreTriageRefresh {
                ordered_urls: vec![url.to_string()],
                triggered_by_job_done,
            },
        );
        for _ in 0..8 {
            let (next_state, effects) = update(state, Msg::Tick);
            if let Some(request_id) = effects.iter().find_map(|effect| match effect {
                Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                _ => None,
            }) {
                let articles = vec![harvester_core::LoadedArticle {
                    url: url.to_string(),
                    source_title: None,
                    prepared_text: std::iter::repeat_n("pre-triage-content", 220)
                        .collect::<Vec<_>>()
                        .join(" "),
                    content_hash: "hash-pre-triage".to_string(),
                    fetched_utc: None,
                }];
                let (next_state, _) = update(
                    next_state,
                    Msg::TriageArticlesLoaded {
                        request_id,
                        articles,
                    },
                );
                return Arc::new(Mutex::new(SharedState { state: next_state }));
            }
            state = next_state;
        }
    }
    panic!("expected pre-triage refresh dispatch for test fixture");
}

fn shared_state_with_single_link() -> Arc<Mutex<SharedState>> {
    let state = AppState::new();
    let (state, _) = update(state, Msg::InputChanged("https://example.com".to_string()));
    let (state, _) = update(state, Msg::UrlsSubmitted);
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id: 1,
            result: JobResultKind::Success,
            content_preview: None,
            extracted_links: vec![ExtractedLink {
                url: "http://example.com/".to_string(),
                text: Some("Example".to_string()),
                kind: LinkKind::Hyperlink,
            }],
            fetched_utc: None,
        },
    );
    let shared = SharedState { state };
    Arc::new(Mutex::new(shared))
}

fn apply_msg(shared: &Arc<Mutex<SharedState>>, msg: Msg) {
    let mut guard = shared.lock().unwrap();
    let (state, _) = update(std::mem::take(&mut guard.state), msg);
    guard.state = state;
}

fn test_handler_with_shared(
    shared: Arc<Mutex<SharedState>>,
) -> (AppEventHandler, mpsc::Receiver<Msg>) {
    let (in_tx, in_rx) = mpsc::channel();
    let _ = in_tx;
    let (out_tx, out_rx) = mpsc::channel();
    let output_dir = std::env::temp_dir();
    let paths = RuntimePaths::new(
        output_dir.clone(),
        output_dir.join("sources.ron"),
        output_dir.join("contexts"),
        output_dir.join("prompts"),
    );
    let platform_handler = Box::new(Win32PlatformHandler);
    let effect_runner = EffectRunner::new(paths.clone(), out_tx.clone(), platform_handler);
    let handler = AppEventHandler::new(
        WindowId::new(1),
        shared,
        in_rx,
        out_tx,
        effect_runner,
        PersistenceWorker::new(paths.state_path.clone()),
        ui::render::TreeRenderState::new(),
    );
    (handler, out_rx)
}

fn test_handler_with_shared_and_temp_state(
    shared: Arc<Mutex<SharedState>>,
) -> (
    AppEventHandler,
    mpsc::Sender<Msg>,
    tempfile::TempDir,
    PathBuf,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let (in_tx, in_rx) = mpsc::channel();
    let (out_tx, _out_rx) = mpsc::channel();
    let output_dir = temp.path().to_path_buf();
    let paths = RuntimePaths::new(
        output_dir.clone(),
        output_dir.join("sources.ron"),
        output_dir.join("contexts"),
        output_dir.join("prompts"),
    );
    let platform_handler = Box::new(Win32PlatformHandler);
    let effect_runner = EffectRunner::new(paths.clone(), out_tx.clone(), platform_handler);
    let handler = AppEventHandler::new(
        WindowId::new(1),
        shared,
        in_rx,
        out_tx,
        effect_runner,
        PersistenceWorker::new(paths.state_path.clone()),
        ui::render::TreeRenderState::new(),
    );
    (handler, in_tx, temp, paths.state_path)
}

fn test_handler_with_outbound() -> (AppEventHandler, mpsc::Receiver<Msg>) {
    test_handler_with_shared(Arc::new(Mutex::new(SharedState::default())))
}

fn test_handler_with_loopback(shared: Arc<Mutex<SharedState>>) -> AppEventHandler {
    let (tx, rx) = mpsc::channel();
    let output_dir = std::env::temp_dir();
    let paths = RuntimePaths::new(
        output_dir.clone(),
        output_dir.join("sources.ron"),
        output_dir.join("contexts"),
        output_dir.join("prompts"),
    );
    let platform_handler = Box::new(Win32PlatformHandler);
    let effect_runner = EffectRunner::new(paths.clone(), tx.clone(), platform_handler);
    AppEventHandler::new(
        WindowId::new(1),
        shared,
        rx,
        tx,
        effect_runner,
        PersistenceWorker::new(paths.state_path.clone()),
        ui::render::TreeRenderState::new(),
    )
}

fn startup_test_paths() -> (tempfile::TempDir, RuntimePaths) {
    let temp = tempfile::tempdir().expect("tempdir");
    let output_dir = temp.path().to_path_buf();
    let paths = RuntimePaths::new(
        output_dir.clone(),
        output_dir.join("sources.ron"),
        output_dir.join("contexts"),
        output_dir.join("prompts"),
    );
    (temp, paths)
}

#[test]
fn tree_item_marker_updates_with_link_state() {
    let shared = shared_state_with_single_link();
    let provider = AppUiStateProvider::new(shared.clone());
    let item_id = link_tree_item_id(1, 0);

    assert_eq!(
        provider.tree_item_marker(WindowId::new(1), item_id),
        TreeItemMarkerKind::None
    );

    apply_msg(
        &shared,
        Msg::LinkToggleRequested {
            job_id: 1,
            link_index: 0,
            checked: true,
        },
    );
    assert_eq!(
        provider.tree_item_marker(WindowId::new(1), item_id),
        TreeItemMarkerKind::Purple
    );

    apply_msg(
        &shared,
        Msg::LinkDownloadCompleted {
            job_id: 1,
            link_index: 0,
            path: PathBuf::from("linked/example.md"),
        },
    );
    assert_eq!(
        provider.tree_item_marker(WindowId::new(1), item_id),
        TreeItemMarkerKind::Green
    );

    apply_msg(
        &shared,
        Msg::LinkDownloadFailed {
            job_id: 1,
            link_index: 0,
            error: "boom".to_string(),
        },
    );
    assert_eq!(
        provider.tree_item_marker(WindowId::new(1), item_id),
        TreeItemMarkerKind::Red
    );

    apply_msg(
        &shared,
        Msg::LinkDeleted {
            job_id: 1,
            link_index: 0,
        },
    );
    assert_eq!(
        provider.tree_item_marker(WindowId::new(1), item_id),
        TreeItemMarkerKind::None
    );
}

#[test]
fn pre_triage_override_persistence_preserves_completed_jobs() {
    let shared = shared_state_with_ready_pre_triage_review();
    let key = {
        let guard = shared.lock().unwrap();
        guard
            .state
            .pre_triage_key_for_job(1)
            .expect("pre-triage key for restored job")
    };
    let (mut handler, in_tx, _temp, state_path) = test_handler_with_shared_and_temp_state(shared);

    in_tx
        .send(Msg::PreTriageDecisionSet {
            key,
            decision: ManualDecision::Exclude,
        })
        .expect("send pre-triage decision");
    handler.process_pending_messages();
    handler.persistence_worker.shutdown();

    let completed = harvester_io::load_completed_jobs(&state_path);
    let overrides = harvester_io::load_pre_triage_overrides(&state_path);
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].url, "https://example.com/pre-triage");
    assert_eq!(overrides.len(), 1);
}

#[test]
fn job_tree_items_show_priority_markers_only_in_triage_results() {
    let shared = shared_state_with_triage_priority(5);
    let provider = AppUiStateProvider::new(shared.clone());
    let item_id = job_tree_item_id(1);

    assert_eq!(
        provider.tree_item_marker(WindowId::new(1), item_id),
        TreeItemMarkerKind::Yellow
    );

    {
        let mut guard = shared.lock().unwrap();
        let (state, _) = update(
            std::mem::take(&mut guard.state),
            Msg::LeftTabSelected { tab: LeftTab::Jobs },
        );
        guard.state = state;
    }

    assert_eq!(
        provider.tree_item_marker(WindowId::new(1), item_id),
        TreeItemMarkerKind::None
    );
}

#[test]
fn job_tree_items_do_not_show_priority_markers_in_triage_review() {
    let shared = shared_state_with_triage_priority(4);
    let provider = AppUiStateProvider::new(shared.clone());
    let item_id = job_tree_item_id(1);

    {
        let mut guard = shared.lock().unwrap();
        let (state, _) = update(
            std::mem::take(&mut guard.state),
            Msg::LeftTabSelected {
                tab: LeftTab::TriageReview,
            },
        );
        guard.state = state;
    }

    assert_eq!(
        provider.tree_item_marker(WindowId::new(1), item_id),
        TreeItemMarkerKind::None
    );
}

#[test]
fn triage_priority_marker_mapping_is_stable() {
    assert_eq!(triage_marker_for_priority(7), TreeItemMarkerKind::Red);
    assert_eq!(triage_marker_for_priority(5), TreeItemMarkerKind::Yellow);
    assert_eq!(triage_marker_for_priority(4), TreeItemMarkerKind::Purple);
    assert_eq!(triage_marker_for_priority(3), TreeItemMarkerKind::Gray);
    assert_eq!(triage_marker_for_priority(2), TreeItemMarkerKind::None);
}

#[test]
fn tree_item_marker_reads_triage_result_without_view_model_rebuild() {
    let shared = shared_state_with_triage_priority(4);
    let provider = AppUiStateProvider::new(shared.clone());
    let item_id = job_tree_item_id(1);

    assert_eq!(
        provider.tree_item_marker(WindowId::new(1), item_id),
        TreeItemMarkerKind::Purple
    );
}

#[test]
fn tree_item_marker_uses_gray_for_priority_three() {
    let shared = shared_state_with_triage_priority(3);
    let provider = AppUiStateProvider::new(shared.clone());
    let item_id = job_tree_item_id(1);

    assert_eq!(
        provider.tree_item_marker(WindowId::new(1), item_id),
        TreeItemMarkerKind::Gray
    );
}

#[test]
fn tree_item_marker_suppresses_low_priority_jobs() {
    let shared = shared_state_with_triage_priority(2);
    let provider = AppUiStateProvider::new(shared.clone());
    let item_id = job_tree_item_id(1);

    assert_eq!(
        provider.tree_item_marker(WindowId::new(1), item_id),
        TreeItemMarkerKind::None
    );
}

#[test]
fn parse_llm_max_concurrency_uses_default_when_missing_or_invalid() {
    assert_eq!(
        parse_llm_max_concurrency_requests(None),
        DEFAULT_LLM_MAX_CONCURRENT_REQUESTS
    );
    assert_eq!(
        parse_llm_max_concurrency_requests(Some("not-a-number")),
        DEFAULT_LLM_MAX_CONCURRENT_REQUESTS
    );
    assert_eq!(
        parse_llm_max_concurrency_requests(Some("")),
        DEFAULT_LLM_MAX_CONCURRENT_REQUESTS
    );
}

#[test]
fn parse_llm_max_concurrency_clamps_to_valid_range() {
    assert_eq!(parse_llm_max_concurrency_requests(Some("1")), 1);
    assert_eq!(parse_llm_max_concurrency_requests(Some("3")), 3);
    assert_eq!(
        parse_llm_max_concurrency_requests(Some("999")),
        MAX_LLM_CONCURRENT_REQUESTS
    );
    assert_eq!(parse_llm_max_concurrency_requests(Some(" 2 ")), 2);
}

#[test]
fn prepare_startup_state_schedules_metadata_load_once() {
    let (_temp, paths) = startup_test_paths();
    let (_state, effects) = prepare_startup_state(
        AppState::new(),
        &paths,
        1200,
        4,
        Some(AiAvailability::Unavailable {
            reason: AiUnavailableReason::MissingApiKey,
        }),
        None,
    );

    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::LoadLlmMetadata))
            .count(),
        1
    );
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::LoadPromptContexts))
            .count(),
        1
    );
}

#[test]
fn assembled_startup_commands_render_before_reveal() {
    let (_temp, paths) = startup_test_paths();
    let (state, _effects) = prepare_startup_state(AppState::new(), &paths, 1200, 4, None, None);
    let view = state.view();
    let window_id = WindowId::new(1);
    let mut tree_render_state = ui::render::TreeRenderState::new();

    let layout_commands = ui::layout::initial_commands(window_id);
    let render_commands = ui::render::render(window_id, &view, &mut tree_render_state);
    let commands =
        assemble_startup_commands(window_id, &view, &mut ui::render::TreeRenderState::new());

    let render_end = layout_commands.len() + render_commands.len();
    let show_window_indexes = commands
        .iter()
        .enumerate()
        .filter_map(|(index, command)| match command {
            PlatformCommand::ShowWindow { .. } => Some(index),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(show_window_indexes, vec![render_end + 1]);
    assert!(matches!(
        commands.get(render_end),
        Some(PlatformCommand::SignalMainWindowUISetupComplete { .. })
    ));
    assert!(matches!(
        commands.get(render_end + 1),
        Some(PlatformCommand::ShowWindow { .. })
    ));
}

#[test]
fn geometry_only_batches_use_layout_only_render_mode() {
    assert_eq!(
        select_render_mode(true, true, false, false, 0),
        Some(RenderMode::LayoutOnly)
    );
    assert_eq!(
        select_render_mode(true, true, false, false, 1),
        Some(RenderMode::Full)
    );
    assert_eq!(
        select_render_mode(true, false, false, false, 0),
        Some(RenderMode::Full)
    );
}

#[test]
fn prompt_lab_stage_button_emits_stage_selected_msg() {
    let (mut handler, rx) = test_handler_with_outbound();
    handler.handle_event(AppEvent::RadioButtonSelected {
        window_id: WindowId::new(1),
        control_id: ui::constants::BTN_STAGE_SUMMARY,
    });
    let msg = rx.recv_timeout(Duration::from_millis(250)).expect("msg");
    assert_eq!(
        msg,
        Msg::PromptLabStageSelected {
            stage: PromptLabStage::Summary
        }
    );
}

#[test]
fn prompt_lab_url_input_emits_url_changed_msg() {
    let (mut handler, rx) = test_handler_with_outbound();
    handler.handle_event(AppEvent::InputTextChanged {
        window_id: WindowId::new(1),
        control_id: ui::constants::INPUT_PROMPT_LAB_URL,
        text: "https://example.com".to_string(),
    });
    let msg = rx.recv_timeout(Duration::from_millis(250)).expect("msg");
    assert_eq!(
        msg,
        Msg::PromptLabUrlInputChanged {
            url: "https://example.com".to_string()
        }
    );
}

#[test]
fn archive_footer_button_emits_archive_clicked() {
    let (mut handler, rx) = test_handler_with_outbound();
    handler.handle_event(AppEvent::ButtonClicked {
        window_id: WindowId::new(1),
        control_id: ui::constants::BUTTON_ARCHIVE,
    });
    let msg = rx.recv_timeout(Duration::from_millis(250)).expect("msg");
    assert_eq!(msg, Msg::ArchiveClicked);
}

#[test]
fn summarize_footer_button_emits_prepare_summaries_clicked() {
    let (mut handler, rx) = test_handler_with_outbound();
    handler.handle_event(AppEvent::ButtonClicked {
        window_id: WindowId::new(1),
        control_id: ui::constants::BUTTON_SUMMARIZE,
    });
    let msg = rx.recv_timeout(Duration::from_millis(250)).expect("msg");
    assert_eq!(msg, Msg::PrepareSummariesClicked);
}

#[test]
fn preview_source_link_emits_open_in_browser_clicked() {
    let (mut handler, rx) = test_handler_with_outbound();
    handler.handle_event(AppEvent::ButtonClicked {
        window_id: WindowId::new(1),
        control_id: ui::constants::BUTTON_PREVIEW_SOURCE_LINK,
    });
    let msg = rx.recv_timeout(Duration::from_millis(250)).expect("msg");
    assert_eq!(msg, Msg::OpenInBrowserClicked);
}

#[test]
fn prompt_lab_action_buttons_emit_expected_msgs() {
    let (mut handler, rx) = test_handler_with_outbound();
    handler.handle_event(AppEvent::ButtonClicked {
        window_id: WindowId::new(1),
        control_id: ui::constants::BTN_PROMPT_LAB_RESOLVE,
    });
    handler.handle_event(AppEvent::ButtonClicked {
        window_id: WindowId::new(1),
        control_id: ui::constants::BTN_PROMPT_LAB_RUN,
    });

    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("resolve"),
        Msg::PromptLabResolveRequested
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250)).expect("run"),
        Msg::PromptLabRunRequested
    );
}

#[test]
fn prompt_lab_template_buttons_emit_expected_msgs() {
    let (mut handler, rx) = test_handler_with_outbound();
    handler.handle_event(AppEvent::CheckBoxToggled {
        window_id: WindowId::new(1),
        control_id: ui::constants::CHK_PROMPT_LAB_TEMPLATE_OPEN,
        checked: true,
    });
    handler.handle_event(AppEvent::ButtonClicked {
        window_id: WindowId::new(1),
        control_id: ui::constants::BTN_PROMPT_LAB_TEMPLATE_APPLY,
    });
    handler.handle_event(AppEvent::ButtonClicked {
        window_id: WindowId::new(1),
        control_id: ui::constants::BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN,
    });
    handler.handle_event(AppEvent::ButtonClicked {
        window_id: WindowId::new(1),
        control_id: ui::constants::BTN_PROMPT_LAB_TEMPLATE_REVERT,
    });
    handler.handle_event(AppEvent::ButtonClicked {
        window_id: WindowId::new(1),
        control_id: ui::constants::BTN_PROMPT_LAB_TEMPLATE_SAVE,
    });

    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("template open"),
        Msg::PromptLabTemplateEditorToggled
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("template apply"),
        Msg::PromptLabTemplateApplyRequested
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("template apply rerun"),
        Msg::PromptLabTemplateApplyAndRerunRequested
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("template revert"),
        Msg::PromptLabTemplateRevertRequested
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("template save"),
        Msg::PromptLabTemplateSaveRequested
    );
}

#[test]
fn prompt_lab_compare_buttons_emit_expected_msgs() {
    let (mut handler, rx) = test_handler_with_outbound();
    handler.handle_event(AppEvent::ButtonClicked {
        window_id: WindowId::new(1),
        control_id: ui::constants::BTN_COMPARE_ADD_CURRENT,
    });
    handler.handle_event(AppEvent::ButtonClicked {
        window_id: WindowId::new(1),
        control_id: ui::constants::BTN_COMPARE_ADD_BASELINE,
    });
    handler.handle_event(AppEvent::ButtonClicked {
        window_id: WindowId::new(1),
        control_id: ui::constants::BTN_COMPARE_RESET_DRAFT,
    });
    handler.handle_event(AppEvent::ButtonClicked {
        window_id: WindowId::new(1),
        control_id: ui::constants::BTN_COMPARE_START,
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("add current"),
        Msg::PromptLabCompareCurrentSettingsCaptured
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("add baseline"),
        Msg::PromptLabCompareBaselineCaptured
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250)).expect("reset"),
        Msg::PromptLabCompareDraftReset
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250)).expect("start"),
        Msg::PromptLabCompareBatchStartRequested
    );
}

#[test]
fn prompt_lab_mode_and_section_buttons_emit_expected_msgs() {
    let (mut handler, rx) = test_handler_with_outbound();
    handler.handle_event(AppEvent::RadioButtonSelected {
        window_id: WindowId::new(1),
        control_id: ui::constants::BTN_PROMPT_LAB_MODE_ADVANCED,
    });
    handler.handle_event(AppEvent::CheckBoxToggled {
        window_id: WindowId::new(1),
        control_id: ui::constants::CHK_PROMPT_LAB_SECTION_COMPARE,
        checked: true,
    });
    handler.handle_event(AppEvent::CheckBoxToggled {
        window_id: WindowId::new(1),
        control_id: ui::constants::CHK_PROMPT_LAB_SECTION_CONTEXT,
        checked: true,
    });
    handler.handle_event(AppEvent::CheckBoxToggled {
        window_id: WindowId::new(1),
        control_id: ui::constants::CHK_PROMPT_LAB_SECTION_TEMPLATE,
        checked: true,
    });
    handler.handle_event(AppEvent::CheckBoxToggled {
        window_id: WindowId::new(1),
        control_id: ui::constants::CHK_PROMPT_LAB_SECTION_RUN_DETAILS,
        checked: true,
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("advanced"),
        Msg::PromptLabAdvancedModeSet { enabled: true }
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("toggle compare"),
        Msg::PromptLabCompareSectionToggled
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("toggle context"),
        Msg::PromptLabContextSectionToggled
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("toggle template"),
        Msg::PromptLabTemplateSectionToggled
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("toggle run details"),
        Msg::PromptLabRunDetailsSectionToggled
    );
}

#[test]
fn left_tab_bar_selection_emits_new_left_tab_variants() {
    let (mut handler, rx) = test_handler_with_outbound();
    handler.handle_event(AppEvent::TabBarSelectionChanged {
        window_id: WindowId::new(1),
        control_id: ui::constants::TAB_BAR_LEFT,
        selected_index: LeftTab::TriageReview.to_index(),
    });
    handler.handle_event(AppEvent::TabBarSelectionChanged {
        window_id: WindowId::new(1),
        control_id: ui::constants::TAB_BAR_LEFT,
        selected_index: LeftTab::TriageResults.to_index(),
    });

    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("triage review tab"),
        Msg::LeftTabSelected {
            tab: LeftTab::TriageReview
        }
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("triage results tab"),
        Msg::LeftTabSelected {
            tab: LeftTab::TriageResults
        }
    );
}

#[test]
fn listbox_x_key_toggles_pre_triage_decision_on_triage_review() {
    let shared = shared_state_with_ready_pre_triage_review();
    apply_msg(
        &shared,
        Msg::LeftTabSelected {
            tab: LeftTab::TriageReview,
        },
    );
    apply_msg(&shared, Msg::JobSelected { job_id: 1 });

    let expected_key = {
        let guard = shared.lock().unwrap();
        guard
            .state
            .pre_triage_key_for_job(1)
            .expect("pre triage key")
    };
    let expected_decision = {
        let guard = shared.lock().unwrap();
        match guard.state.job_filter_status(1) {
            Some(JobFilterStatus::HardExcluded { .. })
            | Some(JobFilterStatus::ManuallyExcluded) => ManualDecision::Include,
            _ => ManualDecision::Exclude,
        }
    };

    let (mut handler, rx) = test_handler_with_shared(shared);
    handler.handle_event(AppEvent::ListBoxItemKeyDown {
        window_id: WindowId::new(1),
        control_id: ui::constants::TREE_JOBS,
        key_code: b'X' as u16,
    });

    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("pre triage decision"),
        Msg::PreTriageDecisionSet {
            key: expected_key,
            decision: expected_decision,
        }
    );
}

#[test]
fn listbox_x_key_does_nothing_outside_pre_triage_review() {
    let shared = shared_state_with_triage_priority(5);
    apply_msg(&shared, Msg::LeftTabSelected { tab: LeftTab::Jobs });
    apply_msg(&shared, Msg::JobSelected { job_id: 1 });

    let (mut handler, rx) = test_handler_with_shared(shared);
    handler.handle_event(AppEvent::ListBoxItemKeyDown {
        window_id: WindowId::new(1),
        control_id: ui::constants::TREE_JOBS,
        key_code: b'X' as u16,
    });

    assert!(rx.recv_timeout(Duration::from_millis(150)).is_err());
}

#[test]
fn jobs_scope_toggle_emits_typed_scope_message() {
    let (mut handler, rx) = test_handler_with_outbound();
    handler.handle_event(AppEvent::ToggleSwitchToggled {
        window_id: WindowId::new(1),
        control_id: ui::constants::TS_JOBS_SCOPE,
        checked: true,
    });
    handler.handle_event(AppEvent::ToggleSwitchToggled {
        window_id: WindowId::new(1),
        control_id: ui::constants::TS_JOBS_SCOPE,
        checked: false,
    });

    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("scope on"),
        Msg::JobListScopeSet {
            scope: JobListScope::SinceCheckpoint
        }
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_millis(250))
            .expect("scope off"),
        Msg::JobListScopeSet {
            scope: JobListScope::All
        }
    );
}

#[test]
fn jobs_search_input_text_changed_updates_query_state() {
    let shared = Arc::new(Mutex::new(SharedState::default()));
    let mut handler = test_handler_with_loopback(shared.clone());

    handler.handle_event(AppEvent::InputTextChanged {
        window_id: WindowId::new(1),
        control_id: ui::constants::INPUT_JOBS_SEARCH,
        text: "kube".to_string(),
    });
    handler.process_pending_messages();

    let view = shared.lock().unwrap().state.view();
    assert_eq!(view.left_pane.jobs_search_query, "kube");
}

#[test]
fn jobs_search_user_typing_does_not_echo_set_input_text() {
    let shared = Arc::new(Mutex::new(SharedState::default()));
    let mut handler = test_handler_with_loopback(shared.clone());

    handler.handle_event(AppEvent::InputTextChanged {
        window_id: WindowId::new(1),
        control_id: ui::constants::INPUT_JOBS_SEARCH,
        text: "a".to_string(),
    });
    handler.process_pending_messages();

    assert_eq!(
        shared
            .lock()
            .unwrap()
            .state
            .view()
            .left_pane
            .jobs_search_query,
        "a"
    );
    assert!(
        !handler.commands.iter().any(|cmd| matches!(
            cmd,
            PlatformCommand::SetInputText {
                control_id,
                text,
                ..
            } if *control_id == ui::constants::INPUT_JOBS_SEARCH && text == "a"
        )),
        "user-originated search text is already in the edit control; echoing it resets the caret"
    );
}

#[test]
fn find_jobs_menu_switches_to_jobs_and_focuses_search() {
    let shared = Arc::new(Mutex::new(SharedState::default()));
    apply_msg(
        &shared,
        Msg::LeftTabSelected {
            tab: LeftTab::PromptLab,
        },
    );
    let mut handler = test_handler_with_loopback(shared.clone());

    handler.handle_event(AppEvent::MenuActionClicked {
        action_id: ui::constants::MENU_ACTION_FIND_JOBS,
    });
    handler.process_pending_messages();

    assert_eq!(shared.lock().unwrap().state.left_tab(), LeftTab::Jobs);
    assert!(handler.commands.iter().any(|cmd| matches!(
        cmd,
        PlatformCommand::SetFocus { control_id, select_all, .. }
            if *control_id == ui::constants::INPUT_JOBS_SEARCH && *select_all
    )));
}

#[test]
fn escape_in_jobs_search_clears_query_and_focuses_list() {
    let shared = Arc::new(Mutex::new(SharedState::default()));
    apply_msg(&shared, Msg::JobsSearchQueryChanged("github".to_string()));
    let mut handler = test_handler_with_loopback(shared.clone());

    handler.handle_event(AppEvent::InputKeyDown {
        window_id: WindowId::new(1),
        control_id: ui::constants::INPUT_JOBS_SEARCH,
        key_code: VK_ESCAPE_CODE,
        modifiers: commanductui::KeyModifiers::default(),
    });
    handler.process_pending_messages();

    let view = shared.lock().unwrap().state.view();
    assert_eq!(view.left_pane.jobs_search_query, "");
    assert!(handler.commands.iter().any(|cmd| matches!(
        cmd,
        PlatformCommand::SetFocus { control_id, select_all, .. }
            if *control_id == ui::constants::TREE_JOBS && !*select_all
    )));
}

#[test]
fn enter_in_jobs_search_selects_first_visible_when_selection_filtered_out() {
    let shared = Arc::new(Mutex::new(SharedState::default()));
    apply_msg(
        &shared,
        Msg::RestoreCompletedJobs(vec![
            completed_job_snapshot("https://hidden.example/a"),
            completed_job_snapshot("https://visible.example/b"),
        ]),
    );
    apply_msg(&shared, Msg::JobSelected { job_id: 1 });
    apply_msg(
        &shared,
        Msg::JobsSearchQueryChanged("visible.example".to_string()),
    );
    let mut handler = test_handler_with_loopback(shared.clone());

    handler.handle_event(AppEvent::InputKeyDown {
        window_id: WindowId::new(1),
        control_id: ui::constants::INPUT_JOBS_SEARCH,
        key_code: VK_RETURN_CODE,
        modifiers: commanductui::KeyModifiers::default(),
    });
    handler.process_pending_messages();

    assert_eq!(shared.lock().unwrap().state.selected_job_id(), Some(2));
    assert!(handler.commands.iter().any(|cmd| matches!(
        cmd,
        PlatformCommand::SetFocus { control_id, select_all, .. }
            if *control_id == ui::constants::TREE_JOBS && !*select_all
    )));
}

/// Regression: when the user types in the search box and hits Enter
/// quickly, the JobsSearchQueryChanged message from InputTextChanged
/// is still sitting unprocessed in msg_tx. The Enter handler must
/// drain pending messages before reading state.view(), or it sees a
/// stale view and picks the wrong first-visible job.
#[test]
fn enter_in_jobs_search_drains_pending_query_before_reading_view() {
    let shared = Arc::new(Mutex::new(SharedState::default()));
    apply_msg(
        &shared,
        Msg::RestoreCompletedJobs(vec![
            completed_job_snapshot("https://hidden.example/a"),
            completed_job_snapshot("https://visible.example/b"),
        ]),
    );
    apply_msg(&shared, Msg::JobSelected { job_id: 1 });
    // Do NOT apply JobsSearchQueryChanged via apply_msg — instead
    // route it through the event handler so it lands in the loopback
    // channel unprocessed, simulating a fast keystroke+Enter.
    let mut handler = test_handler_with_loopback(shared.clone());

    handler.handle_event(AppEvent::InputTextChanged {
        window_id: WindowId::new(1),
        control_id: ui::constants::INPUT_JOBS_SEARCH,
        text: "visible.example".to_string(),
    });
    // Immediately fire Enter without draining — the dispatcher invariant
    // (drain at top of handle_event) must apply the prior query before the
    // Enter branch reads the view.
    handler.handle_event(AppEvent::InputKeyDown {
        window_id: WindowId::new(1),
        control_id: ui::constants::INPUT_JOBS_SEARCH,
        key_code: VK_RETURN_CODE,
        modifiers: commanductui::KeyModifiers::default(),
    });
    handler.process_pending_messages();

    // Without the fix selected_job_id stays 1 (stale, invisible under
    // the new query => no-op). With the fix, it's 2 (first visible).
    assert_eq!(shared.lock().unwrap().state.selected_job_id(), Some(2));
    assert!(handler.commands.iter().any(|cmd| matches!(
        cmd,
        PlatformCommand::SetFocus { control_id, select_all, .. }
            if *control_id == ui::constants::TREE_JOBS && !*select_all
    )));
}

#[test]
fn enter_in_jobs_search_focuses_tree_when_filter_is_empty() {
    let shared = Arc::new(Mutex::new(SharedState::default()));
    apply_msg(
        &shared,
        Msg::RestoreCompletedJobs(vec![
            completed_job_snapshot("https://example.com/a"),
            completed_job_snapshot("https://example.com/b"),
        ]),
    );
    apply_msg(&shared, Msg::JobSelected { job_id: 1 });
    apply_msg(&shared, Msg::JobsSearchQueryChanged("no-match".to_string()));
    let mut handler = test_handler_with_loopback(shared.clone());

    handler.handle_event(AppEvent::InputKeyDown {
        window_id: WindowId::new(1),
        control_id: ui::constants::INPUT_JOBS_SEARCH,
        key_code: VK_RETURN_CODE,
        modifiers: commanductui::KeyModifiers::default(),
    });
    handler.process_pending_messages();

    // Filter matches nothing → no JobSelected sent, selection unchanged.
    assert_eq!(shared.lock().unwrap().state.selected_job_id(), Some(1));
    // Focus command is enqueued via queue_focus_after_render, which
    // lands in pending_focus_after_render (not yet flushed to commands
    // because no message triggered a render cycle).
    assert!(handler
        .pending_focus_after_render
        .iter()
        .any(|pf| pf.control_id == ui::constants::TREE_JOBS && !pf.select_all));
}

#[test]
fn enter_in_jobs_search_focuses_tree_when_selection_already_visible() {
    let shared = Arc::new(Mutex::new(SharedState::default()));
    apply_msg(
        &shared,
        Msg::RestoreCompletedJobs(vec![
            completed_job_snapshot("https://example.com/a"),
            completed_job_snapshot("https://other.example/b"),
        ]),
    );
    apply_msg(&shared, Msg::JobSelected { job_id: 1 });
    apply_msg(
        &shared,
        Msg::JobsSearchQueryChanged("example.com/a".to_string()),
    );
    let mut handler = test_handler_with_loopback(shared.clone());

    handler.handle_event(AppEvent::InputKeyDown {
        window_id: WindowId::new(1),
        control_id: ui::constants::INPUT_JOBS_SEARCH,
        key_code: VK_RETURN_CODE,
        modifiers: commanductui::KeyModifiers::default(),
    });
    handler.process_pending_messages();

    // The selected job (1) is visible in the filter → no JobSelected.
    assert_eq!(shared.lock().unwrap().state.selected_job_id(), Some(1));
    assert!(handler
        .pending_focus_after_render
        .iter()
        .any(|pf| pf.control_id == ui::constants::TREE_JOBS && !pf.select_all));
}
