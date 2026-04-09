    use super::*;
    use harvester_core::Stage;
    use harvester_core::{
        JobOrigin, LinkDownloadState, LinkRowView, PromptLabRunId, PromptLabRunSummaryView,
        PromptLabView,
    };
    use std::path::PathBuf;
    use std::sync::Once;

    fn init_logging() {
        static INIT: Once = Once::new();
        INIT.call_once(engine_logging::initialize_for_tests);
    }

    fn make_job(
        job_id: u64,
        url: &str,
        stage: Stage,
        outcome: Option<JobResultKind>,
        tokens: Option<u32>,
        bytes: Option<u64>,
    ) -> JobRowView {
        JobRowView {
            job_id,
            url: url.to_string(),
            stage,
            outcome,
            tokens,
            bytes,
            link_count: 0,
            downloaded_link_count: 0,
            links: Vec::new(),
            triage_annotation: None,
            has_summary: false,
            summary_title: None,
            filter_status: None,
            has_analysis: false,
            origin: JobOrigin::Direct,
            is_since_checkpoint: false,
        }
    }

    fn make_link_row(index: u32, label: &str, download_state: LinkDownloadState) -> LinkRowView {
        LinkRowView {
            index,
            url: format!("https://links.example/{index}"),
            label: label.to_string(),
            kind: LinkKind::Hyperlink,
            download_state,
            age_suspect: false,
        }
    }

    fn make_view(jobs: Vec<JobRowView>) -> AppViewModel {
        let mut view = AppViewModel {
            job_count: jobs.len(),
            jobs,
            ..AppViewModel::default()
        };
        view.left_pane.job_list_scope = JobListScope::All;
        view
    }

    fn completed_prompt_lab_view() -> PromptLabView {
        PromptLabView {
            visible: true,
            latest_run: Some(PromptLabRunSummaryView {
                run_id: PromptLabRunId(1),
                stage: PromptLabStage::Summary,
                status_label: "completed",
                output_json: Some("{\"ok\":true}".to_string()),
                failure_reason: None,
                input_tokens: Some(10),
                output_tokens: Some(20),
                cost_microdollars: Some(30),
                wall_ms: Some(40),
                resolved_model: Some(harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI.to_string()),
                parse_ok: Some(true),
                cache_status: Some("miss".to_string()),
            }),
            ..PromptLabView::default()
        }
    }

    #[test]
    fn preview_context_exposes_source_status_and_ignores_analytics_fields() {
        init_logging();
        let header = PreviewHeaderView {
            domain: "example.com".to_string(),
            tokens: Some(1234),
            bytes: Some(2048),
            stage: Stage::Done,
            outcome: Some(JobResultKind::Success),
            heading_count: 8,
            link_density: 0.0,
            nav_heavy: false,
        };
        let context = format_preview_context(&header);
        assert_eq!(context.source_label, "example.com");
        assert_eq!(context.status_label, "Done");
        assert_eq!(context.attention_label, None);
    }

    #[test]
    fn preview_context_appends_nav_heavy_indicator() {
        init_logging();
        let header = PreviewHeaderView {
            domain: "dense.example".to_string(),
            tokens: None,
            bytes: None,
            stage: Stage::Converting,
            outcome: None,
            heading_count: 0,
            link_density: 1.0,
            nav_heavy: true,
        };
        let context = format_preview_context(&header);
        assert_eq!(context.source_label, "dense.example");
        assert_eq!(context.status_label, "Converting");
        assert_eq!(context.attention_label.as_deref(), Some("navigation-heavy"));
    }

    #[test]
    fn preview_header_text_override_wins_over_article_header() {
        init_logging();
        let window_id = WindowId::new(1);
        let mut tree_state = TreeRenderState::new();
        let view = AppViewModel {
            preview_header: Some(PreviewHeaderView {
                domain: "example.com".to_string(),
                tokens: Some(123),
                bytes: Some(456),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                heading_count: 2,
                link_density: 0.0,
                nav_heavy: false,
            }),
            preview_context: Some(PreviewContextView {
                source_label: "example.com".to_string(),
                status_label: "Done".to_string(),
                attention_label: None,
            }),
            preview_header_text: Some("Executive Briefing | 3 articles | Done".to_string()),
            ..AppViewModel::default()
        };

        let commands = render(window_id, &view, &mut tree_state);

        let header = commands.iter().find_map(|cmd| match cmd {
            PlatformCommand::SetControlText {
                control_id, text, ..
            } if *control_id == LABEL_PREVIEW_HEADER => Some(text.as_str()),
            _ => None,
        });
        assert_eq!(header, Some("Executive Briefing | 3 articles | Done"));
        assert_eq!(control_text(&commands, LABEL_PREVIEW_SOURCE), Some(""));
        assert_eq!(control_text(&commands, LABEL_PREVIEW_STATUS), Some(""));
        assert_eq!(control_text(&commands, LABEL_PREVIEW_ATTENTION), Some(""));
    }

    #[test]
    fn preview_metadata_clears_when_no_article_is_selected() {
        init_logging();
        let window_id = WindowId::new(2);
        let mut tree_state = TreeRenderState::new();
        let view = make_view(vec![]);

        let commands = render(window_id, &view, &mut tree_state);

        assert_eq!(control_text(&commands, LABEL_PREVIEW_HEADER), Some(""));
        assert_eq!(control_text(&commands, LABEL_PREVIEW_SOURCE), Some(""));
        assert_eq!(control_text(&commands, LABEL_PREVIEW_STATUS), Some(""));
        assert_eq!(control_text(&commands, LABEL_PREVIEW_ATTENTION), Some(""));
    }

    #[test]
    fn header_texts_do_not_reemit_when_unchanged() {
        init_logging();
        let window_id = WindowId::new(3);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![make_job(
            1,
            "https://epochai.substack.com/p/what",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        )]);
        view.left_pane.left_tab = LeftTab::TriageResults;
        view.left_pane.job_list_scope = JobListScope::SinceCheckpoint;
        view.jobs[0].triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 1,
            category: "keep".to_string(),
            tags: vec![],
        });

        let _ = render(window_id, &view, &mut tree_state);
        let commands = render(window_id, &view, &mut tree_state);

        assert!(!commands.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlText { control_id, .. }
                    if *control_id == LABEL_JOBS_HEADER_TITLE
                        || *control_id == LABEL_JOBS_HEADER_META
                        || *control_id == LABEL_PREVIEW_HEADER
                        || *control_id == LABEL_PREVIEW_SOURCE
                        || *control_id == LABEL_PREVIEW_STATUS
                        || *control_id == LABEL_PREVIEW_ATTENTION
            )
        }));
    }

    #[test]
    fn tree_updates_text_without_repopulate_on_progress_change() {
        init_logging();
        let window_id = WindowId::new(1);
        let mut tree_state = TreeRenderState::new();

        let view_initial = make_view(vec![make_job(
            1,
            "https://example.com",
            Stage::Queued,
            None,
            None,
            None,
        )]);
        let commands_initial = render(window_id, &view_initial, &mut tree_state);
        assert!(commands_initial
            .iter()
            .any(|cmd| matches!(cmd, PlatformCommand::PopulateListBox { .. })));

        let view_updated = make_view(vec![make_job(
            1,
            "https://example.com",
            Stage::Downloading,
            None,
            Some(100),
            Some(2048),
        )]);
        let commands_updated = render(window_id, &view_updated, &mut tree_state);
        let populated = commands_updated
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::PopulateListBox { items, .. } => Some(items),
                _ => None,
            })
            .expect("updated list emitted");
        assert_eq!(populated.len(), 1);
        assert_eq!(populated[0].id, ListBoxItemId(1));
        assert_eq!(populated[0].title, "example.com");
        assert_eq!(populated[0].badges[0].text, "Fetch");
        assert_eq!(populated[0].metadata, "example.com · 100 · 2.0 KB");
    }

    #[test]
    fn triage_review_items_show_indirect_badge_and_disabled_state() {
        let mut job = make_job(1, "https://example.com", Stage::Done, None, None, None);
        job.summary_title = Some("Example headline".to_string());
        job.filter_status = Some(JobFilterStatus::ManuallyExcluded);
        job.origin = JobOrigin::Indirect { source_job_id: 9 };
        job.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 4,
            category: "security".to_string(),
            tags: vec![],
        });

        let item = build_list_box_item(LeftTab::TriageReview, &job);

        assert!(!item.enabled);
        assert_eq!(item.badges.len(), 2);
        assert_eq!(item.badges[0].text, "Excluded");
        assert_eq!(item.badges[1].text, "Indirect");
        assert_eq!(item.badges[1].style, StyleId::BadgeIndirect);
        assert_eq!(item.metadata, "Security · example.com");
    }

    #[test]
    fn triage_results_items_show_priority_and_category_badges() {
        let mut job = make_job(1, "https://example.com", Stage::Done, None, None, None);
        job.summary_title = Some("Example headline".to_string());
        job.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "business".to_string(),
            tags: vec!["tag-a".to_string()],
        });

        let item = build_list_box_item(LeftTab::TriageResults, &job);

        assert_eq!(item.badges.len(), 2);
        assert_eq!(item.badges[0].text, "P5");
        assert_eq!(item.badges[0].style, StyleId::BadgePriorityHigh);
        assert_eq!(item.badges[1].text, "Business");
        assert_eq!(item.badges[1].style, StyleId::BadgeCategory);
        assert_eq!(item.metadata, "example.com · 1 tag");
    }

    #[test]
    fn tree_repopulates_when_structure_changes() {
        init_logging();
        let window_id = WindowId::new(2);
        let mut tree_state = TreeRenderState::new();

        let view_initial = make_view(vec![make_job(
            1,
            "https://example.com",
            Stage::Queued,
            None,
            None,
            None,
        )]);
        let _ = render(window_id, &view_initial, &mut tree_state);

        let view_added = make_view(vec![
            make_job(1, "https://example.com", Stage::Queued, None, None, None),
            make_job(2, "https://two.example", Stage::Queued, None, None, None),
        ]);
        let commands_added = render(window_id, &view_added, &mut tree_state);
        assert!(commands_added
            .iter()
            .any(|cmd| matches!(cmd, PlatformCommand::PopulateListBox { .. })));
    }

    #[test]
    fn links_folder_and_show_more_children_rendered() {
        init_logging();
        let link = make_link_row(
            0,
            "Example",
            LinkDownloadState::Downloaded {
                path: PathBuf::from("linked/example.md"),
            },
        );
        let job = JobRowView {
            job_id: 42,
            url: "https://example.com".to_string(),
            stage: Stage::Done,
            outcome: Some(JobResultKind::Success),
            tokens: None,
            bytes: None,
            link_count: 4,
            downloaded_link_count: 1,
            links: vec![link],
            triage_annotation: None,
            has_summary: false,
            summary_title: None,
            filter_status: None,
            has_analysis: false,
            origin: JobOrigin::Direct,
            is_since_checkpoint: false,
        };
        let view = make_view(vec![job]);
        let items = build_job_tree(&view);
        let job_item = &items[0];
        assert_eq!(job_item.children.len(), 1);
        let folder = &job_item.children[0];
        assert_eq!(folder.text, "Links (4)");
        assert_eq!(folder.state, CheckState::Hidden);
        assert_eq!(folder.children.len(), 2);
        assert_eq!(folder.children[0].id, link_tree_item_id(42, 0));
        assert_eq!(folder.children[0].state, CheckState::Hidden);
        let show_more = &folder.children[1];
        assert_eq!(show_more.id, links_show_more_tree_item_id(42));
        assert_eq!(show_more.text, "(show more… 3 remaining)");
        assert_eq!(show_more.state, CheckState::Hidden);
    }

    #[test]
    fn normalize_windows_newlines_handles_various_sequences() {
        assert_eq!(normalize_windows_newlines("line1\nline2"), "line1\r\nline2");
        assert_eq!(normalize_windows_newlines("line1\rline2"), "line1\r\nline2");
        assert_eq!(
            normalize_windows_newlines("line1\r\nline2"),
            "line1\r\nline2"
        );
        assert_eq!(
            normalize_windows_newlines("line1\r\nline2\nline3\rline4"),
            "line1\r\nline2\r\nline3\r\nline4"
        );
    }

    #[test]
    fn preview_text_is_sent_as_rtf_to_rich_edit() {
        init_logging();
        let window_id = WindowId::new(3);
        let mut tree_state = TreeRenderState::new();
        let mut view = AppViewModel::default();
        view.right_pane.summary_markdown = Some("first\nsecond\r\nthird\rfourth".to_string());

        let commands = render(window_id, &view, &mut tree_state);
        let viewer_text = commands
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::SetRichEditContent { rtf_text, .. } => Some(rtf_text),
                _ => None,
            })
            .expect("SetRichEditContent emitted");
        assert!(viewer_text.contains("first"));
        assert!(viewer_text.contains("second"));
    }

    #[test]
    fn shape_adds_blank_line_before_heading() {
        let shaped = shape_for_viewer("text\n# Heading");
        assert_eq!(shaped, "text\n\n# Heading");
    }

    #[test]
    fn shape_heading_already_preceded_by_blank_not_doubled() {
        let shaped = shape_for_viewer("text\n\n# Heading");
        assert_eq!(shaped, "text\n\n# Heading");
    }

    #[test]
    fn shape_bullet_normalized() {
        let shaped = shape_for_viewer("- item");
        assert_eq!(shaped, "• item");
    }

    #[test]
    fn shape_bold_markers_stripped() {
        let shaped = shape_for_viewer("**term**");
        assert_eq!(shaped, "term");
    }

    #[test]
    fn shape_blank_line_runs_capped() {
        let shaped = shape_for_viewer("a\n\n\n\nb");
        assert_eq!(shaped, "a\n\n\nb");
    }

    #[test]
    fn shape_length_guard_truncates() {
        let source = "x".repeat(MAX_VIEWER_CHARS + 10);
        let shaped = shape_for_viewer(&source);
        assert!(shaped.ends_with(VIEWER_TRUNCATE_MARKER));
        assert_eq!(shaped.chars().count(), MAX_VIEWER_CHARS);
    }

    #[test]
    fn render_preview_uses_rtf_converter() {
        init_logging();
        let window_id = WindowId::new(6);
        let mut tree_state = TreeRenderState::new();
        let view = AppViewModel {
            preview_text: Some("## Heading".to_string()),
            ..Default::default()
        };

        let commands = render(window_id, &view, &mut tree_state);
        let viewer_text = commands
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::SetRichEditContent { rtf_text, .. } => Some(rtf_text),
                _ => None,
            })
            .expect("SetRichEditContent emitted");
        assert!(viewer_text.contains("\\b"));
    }

    #[test]
    fn render_preview_marks_bold_in_rtf() {
        init_logging();
        let window_id = WindowId::new(7);
        let mut tree_state = TreeRenderState::new();
        let mut view = AppViewModel::default();
        view.right_pane.summary_markdown = Some("**bold**".to_string());

        let commands = render(window_id, &view, &mut tree_state);
        let viewer_text = commands
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::SetRichEditContent { rtf_text, .. } => Some(rtf_text),
                _ => None,
            })
            .expect("SetRichEditContent emitted");
        assert!(viewer_text.contains("\\b "));
        assert!(viewer_text.contains("\\b0 "));
    }

    #[test]
    fn render_preview_idempotent_when_text_unchanged() {
        init_logging();
        let window_id = WindowId::new(8);
        let mut tree_state = TreeRenderState::new();
        let view = AppViewModel {
            preview_text: Some("same".to_string()),
            ..Default::default()
        };
        let _ = render(window_id, &view, &mut tree_state);
        let commands = render(window_id, &view, &mut tree_state);
        assert!(!commands
            .iter()
            .any(|cmd| matches!(cmd, PlatformCommand::SetRichEditContent { .. })));
    }

    #[test]
    fn render_preview_truncation_adds_marker() {
        init_logging();
        let window_id = WindowId::new(9);
        let mut tree_state = TreeRenderState::new();
        let long_text = "x".repeat(MAX_VIEWER_CHARS + 1);
        let mut view = AppViewModel::default();
        view.right_pane.summary_markdown = Some(long_text);
        let commands = render(window_id, &view, &mut tree_state);
        let viewer_text = commands
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::SetRichEditContent { rtf_text, .. } => Some(rtf_text),
                _ => None,
            })
            .expect("SetRichEditContent emitted");
        assert!(viewer_text.contains(RTF_TRUNCATE_MARKER));
    }

    #[test]
    fn splitter_resize_keeps_input_panel_fixed() {
        init_logging();
        let window_id = WindowId::new(5);
        let mut tree_state = TreeRenderState::new();
        let view = AppViewModel {
            left_panel_width: 760,
            input_panel_visible: true,
            ..Default::default()
        };

        let commands = render(window_id, &view, &mut tree_state);
        let rules = commands
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::DefineLayout { rules, .. } => Some(rules),
                _ => None,
            })
            .expect("DefineLayout emitted");

        let input_width = rules
            .iter()
            .find(|rule| rule.control_id == PANEL_INPUT)
            .and_then(|rule| rule.fixed_size)
            .expect("PANEL_INPUT fixed size");
        let left_width = rules
            .iter()
            .find(|rule| rule.control_id == PANEL_LEFT)
            .and_then(|rule| rule.fixed_size)
            .expect("PANEL_LEFT fixed size");
        let jobs_fill = rules
            .iter()
            .find(|rule| rule.control_id == PANEL_JOBS)
            .map(|rule| rule.fixed_size)
            .expect("PANEL_JOBS rule present");

        assert_eq!(input_width, harvester_core::INPUT_PANEL_FIXED_WIDTH);
        assert_eq!(left_width, 760);
        assert_eq!(jobs_fill, None, "PANEL_JOBS should Fill its parent");
    }

    #[test]
    fn initial_render_emits_layout_for_default_view() {
        init_logging();
        let window_id = WindowId::new(51);
        let mut tree_state = TreeRenderState::new();

        let commands = render(window_id, &AppViewModel::default(), &mut tree_state);

        assert!(commands
            .iter()
            .any(|cmd| matches!(cmd, PlatformCommand::DefineLayout { .. })));
    }

    #[test]
    fn render_layout_only_skips_tree_and_preview_updates() {
        init_logging();
        let window_id = WindowId::new(6);
        let mut tree_state = TreeRenderState::new();
        let view = AppViewModel {
            left_panel_width: 760,
            input_panel_visible: true,
            ..Default::default()
        };
        let _ = render(window_id, &view, &mut tree_state);

        let mut layout = layout_view_from_app_view(&view);
        layout.left_panel_width = 720;
        let commands = render_layout_only(window_id, &layout, &mut tree_state);

        assert!(commands
            .iter()
            .any(|cmd| matches!(cmd, PlatformCommand::DefineLayout { .. })));
        assert!(!commands.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::PopulateTreeView { .. }
                    | PlatformCommand::UpdateTreeItemText { .. }
                    | PlatformCommand::UpdateTreeItemVisualState { .. }
                    | PlatformCommand::SetRichEditContent { .. }
            )
        }));
    }

    #[test]
    fn job_without_summary_gets_tree_item_disabled_style_override() {
        init_logging();
        let mut job = make_job(1, "https://example.com", Stage::Done, None, None, None);
        job.has_summary = false;
        let view = make_view(vec![job]);
        let items = build_job_tree(&view);
        assert_eq!(items[0].style_override, Some(StyleId::TreeItemDisabled));
    }

    #[test]
    fn job_with_summary_has_no_style_override() {
        init_logging();
        let mut job = make_job(1, "https://example.com", Stage::Done, None, None, None);
        job.has_summary = true;
        job.summary_title = Some("Summary headline".to_string());
        let view = make_view(vec![job]);
        let items = build_job_tree(&view);
        assert_eq!(items[0].style_override, None);
    }

    #[test]
    fn format_job_row_prefers_summary_headline_layout_after_summary() {
        let mut job = make_job(
            7,
            "https://example.com/path?q=1",
            Stage::Done,
            Some(JobResultKind::Success),
            Some(123),
            Some(456),
        );
        job.has_summary = true;
        job.summary_title = Some("Headline from summary".to_string());
        job.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 4,
            category: "security".to_string(),
            tags: vec!["tag1".to_string()],
        });

        let row = format_job_row_legacy(&job);
        // Triage annotation must NOT appear in the Jobs tab — row is stable pre/post triage.
        assert_eq!(
            row,
            "Headline from summary — OK · example.com · 123 tok · 456 B"
        );
        assert!(!row.contains("P4"));
        assert!(!row.contains("[#7]"));
        assert!(!row.contains("https://example.com/path?q=1"));
    }

    #[test]
    fn format_job_row_keeps_legacy_layout_before_summary_exists() {
        let mut job = make_job(
            9,
            "https://example.com/path",
            Stage::Done,
            Some(JobResultKind::Success),
            Some(100),
            Some(200),
        );
        job.has_summary = false;
        job.summary_title = Some("Should not be used yet".to_string());

        let row = format_job_row_legacy(&job);
        assert_eq!(row, "example.com/path — OK · 100 tok · 200 B");
        assert!(!row.contains("https://"));
    }

    #[test]
    fn format_job_row_legacy_compacts_long_url_when_no_summary_exists() {
        let job = make_job(
            10,
            "https://example.com/very-long-section/2026/04/03/story-name?utm_source=test",
            Stage::Downloading,
            None,
            None,
            Some(2_048),
        );

        let row = format_job_row_legacy(&job);
        assert_eq!(
            row,
            "example.com/very-long-section/.../story-name — Fetch · 2.0 KB"
        );
        assert!(!row.contains('?'));
    }

    // ── Per-tab formatter tests ───────────────────────────────────────────────

    #[test]
    fn format_job_row_triage_review_shows_review_needed_prefix() {
        let mut job = make_job(
            1,
            "https://example.com/a",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job.has_summary = true;
        job.summary_title = Some("Article Title".to_string());
        job.filter_status = Some(JobFilterStatus::ReviewNeeded { reasons: vec![] });
        let row = format_job_row_triage_review(&job);
        assert!(
            row.starts_with("[REVIEW NEEDED] "),
            "expected review prefix, got: {row}"
        );
        assert!(row.contains("Article Title"));
    }

    #[test]
    fn format_job_row_triage_review_shows_excluded_prefix() {
        let mut job = make_job(2, "https://example.com/b", Stage::Done, None, None, None);
        job.filter_status = Some(JobFilterStatus::ManuallyExcluded);
        let row = format_job_row_triage_review(&job);
        assert!(
            row.starts_with("[EXCLUDED] "),
            "expected excluded prefix, got: {row}"
        );
    }

    #[test]
    fn format_job_row_triage_results_shows_annotation() {
        let mut job = make_job(
            3,
            "https://example.com/c",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job.has_summary = true;
        job.summary_title = Some("Summary Headline".to_string());
        job.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 2,
            category: "tech".to_string(),
            tags: vec!["ai".to_string(), "ml".to_string()],
        });
        let row = format_job_row_triage_results(&job);
        assert_eq!(row, "Summary Headline — P2 Tech · example.com · 2 tags");
    }

    #[test]
    fn format_job_row_triage_results_no_annotation_shows_placeholder() {
        let mut job = make_job(4, "https://example.com/d", Stage::Done, None, None, None);
        job.triage_annotation = None;
        let row = format_job_row_triage_results(&job);
        assert_eq!(row, "example.com/d — No triage");
    }

    #[test]
    fn format_job_row_triage_results_uses_slug_and_compact_tag_count_without_summary() {
        let mut job = make_job(
            5,
            "https://epochai.substack.com/p/hyperscaler-capex-has-quadrupled",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "business".to_string(),
            tags: vec![
                "capex".to_string(),
                "resource-grab".to_string(),
                "hyperscalers".to_string(),
                "sovereign-ai".to_string(),
            ],
        });

        let row = format_job_row_triage_results(&job);
        assert_eq!(
            row,
            "Hyperscaler Capex Has Quadrupled — P5 Business · epochai.substack.com · 4 tags"
        );
    }

    // ── Scope filter tests ────────────────────────────────────────────────────

    #[test]
    fn scope_since_checkpoint_filters_jobs_in_tree() {
        init_logging();

        let mut job_in = make_job(
            1,
            "https://a.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job_in.is_since_checkpoint = true;
        job_in.has_summary = true;
        job_in.summary_title = Some("In Scope".to_string());

        let mut job_out = make_job(
            2,
            "https://b.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job_out.is_since_checkpoint = false;
        job_out.has_summary = true;
        job_out.summary_title = Some("Out of Scope".to_string());

        let mut view = make_view(vec![job_in, job_out]);
        view.left_pane.job_list_scope = JobListScope::SinceCheckpoint;

        let populated = build_list_box_items(&view);

        assert_eq!(populated.len(), 1, "only the in-scope job should appear");
        assert!(
            populated[0].title.contains("In Scope"),
            "wrong item: {}",
            populated[0].title
        );
    }

    #[test]
    fn scope_all_shows_all_jobs() {
        init_logging();

        let mut job1 = make_job(
            1,
            "https://a.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job1.is_since_checkpoint = true;
        job1.has_summary = true;
        job1.summary_title = Some("Job A".to_string());

        let mut job2 = make_job(
            2,
            "https://b.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job2.is_since_checkpoint = false;
        job2.has_summary = true;
        job2.summary_title = Some("Job B".to_string());

        let mut view = make_view(vec![job1, job2]);
        view.left_pane.job_list_scope = JobListScope::All;

        let populated = build_list_box_items(&view);

        assert_eq!(populated.len(), 2, "all jobs should appear with All scope");
    }

    #[test]
    fn triage_results_tab_sorts_by_priority_descending() {
        // Jobs arrive in job_id order (low priority first), but TriageResults should
        // show highest priority first once triage has settled.
        init_logging();

        let mut low = make_job(
            1,
            "https://low.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        low.has_summary = true;
        low.summary_title = Some("Low Priority".to_string());
        low.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 2,
            category: "misc".to_string(),
            tags: vec![],
        });

        let mut high = make_job(
            2,
            "https://high.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        high.has_summary = true;
        high.summary_title = Some("High Priority".to_string());
        high.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "tech".to_string(),
            tags: vec![],
        });

        // View model has jobs in job_id order (low=1, high=2) — stable, not triage-sorted.
        let mut view = make_view(vec![low, high]);
        view.left_pane.left_tab = LeftTab::TriageResults;

        let populated = build_list_box_items(&view);

        assert_eq!(populated.len(), 2);
        // TriageResults render must reorder: highest priority (P5) first.
        assert!(
            populated[0].title.contains("High Priority"),
            "first item should be High Priority, got: {}",
            populated[0].title
        );
        assert!(
            populated[1].title.contains("Low Priority"),
            "second item should be Low Priority, got: {}",
            populated[1].title
        );
    }

    #[test]
    fn triage_results_tab_keeps_stable_order_while_triage_in_progress() {
        init_logging();

        let low = make_job(
            1,
            "https://low.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );

        let mut high = make_job(
            2,
            "https://high.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        high.has_summary = true;
        high.summary_title = Some("High Priority".to_string());
        high.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "tech".to_string(),
            tags: vec![],
        });

        let mut view = make_view(vec![low, high]);
        view.left_pane.left_tab = LeftTab::TriageResults;
        view.triage_progress = Some("Triaging 1/2 articles...".to_string());

        let populated = build_list_box_items(&view);

        assert_eq!(populated.len(), 2);
        assert!(
            populated[0].title.starts_with("low.com"),
            "first should stay job 1 while triage is running, got: {}",
            populated[0].title
        );
        assert!(
            populated[0].metadata.contains("low.com"),
            "first should still show the source label, got: {}",
            populated[0].metadata
        );
        assert!(
            populated[0].metadata.contains("0 tags"),
            "first should still show zero tags, got: {}",
            populated[0].metadata
        );
        assert!(
            populated[1].title.contains("High Priority"),
            "second should stay job 2 while triage is running, got: {}",
            populated[1].title
        );
    }

    #[test]
    fn triage_results_in_progress_updates_rows_without_repopulating_tree() {
        init_logging();
        let window_id = WindowId::new(65);
        let mut tree_state = TreeRenderState::new();

        let low_initial = make_job(
            1,
            "https://low.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );

        let mut high_initial = make_job(
            2,
            "https://high.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        high_initial.has_summary = true;
        high_initial.summary_title = Some("High Priority".to_string());
        high_initial.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "tech".to_string(),
            tags: vec![],
        });

        let mut initial_view = make_view(vec![low_initial, high_initial]);
        initial_view.left_pane.left_tab = LeftTab::TriageResults;
        initial_view.triage_progress = Some("Triaging 1/2 articles...".to_string());
        let _ = render(window_id, &initial_view, &mut tree_state);

        let mut low_updated = make_job(
            1,
            "https://low.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        low_updated.has_summary = true;
        low_updated.summary_title = Some("Now Highest".to_string());
        low_updated.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 6,
            category: "finance".to_string(),
            tags: vec![],
        });

        let mut high_updated = make_job(
            2,
            "https://high.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        high_updated.has_summary = true;
        high_updated.summary_title = Some("High Priority".to_string());
        high_updated.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "tech".to_string(),
            tags: vec![],
        });

        let mut updated_view = make_view(vec![low_updated, high_updated]);
        updated_view.left_pane.left_tab = LeftTab::TriageResults;
        updated_view.triage_progress = Some("Triaging 2/2 articles...".to_string());

        let cmds = render(window_id, &updated_view, &mut tree_state);

        let populated = cmds
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::PopulateListBox { items, .. } => Some(items),
                _ => None,
            })
            .expect("updated list emitted");
        assert_eq!(populated.len(), 2);
        assert_eq!(populated[0].id, ListBoxItemId(1));
        assert!(populated[0].title.contains("Now Highest"));
        assert!(populated[0].metadata.contains("low.com"));
        assert!(populated[0].metadata.contains("0 tags"));
    }

    #[test]
    fn jobs_tab_stable_order_unaffected_by_triage_priority() {
        // Jobs tab must show items in job_id order, not reordered by triage priority.
        init_logging();

        let mut low = make_job(
            1,
            "https://low.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        low.has_summary = true;
        low.summary_title = Some("Low Priority".to_string());
        low.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 2,
            category: "misc".to_string(),
            tags: vec![],
        });

        let mut high = make_job(
            2,
            "https://high.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        high.has_summary = true;
        high.summary_title = Some("High Priority".to_string());
        high.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "tech".to_string(),
            tags: vec![],
        });

        let mut view = make_view(vec![low, high]);
        view.left_pane.left_tab = LeftTab::Jobs; // stable order

        let populated = build_list_box_items(&view);

        assert_eq!(populated.len(), 2);
        // Jobs tab must preserve insertion order (job_id 1 first, then job_id 2).
        assert!(
            populated[0].title.contains("Low Priority"),
            "first should be Low Priority (job 1), got: {}",
            populated[0].title
        );
        assert!(
            populated[1].title.contains("High Priority"),
            "second should be High Priority (job 2), got: {}",
            populated[1].title
        );
    }

    // ── Per-tab style policy tests ────────────────────────────────────────────

    #[test]
    fn jobs_tab_applies_disabled_style_when_no_summary() {
        assert_eq!(
            job_row_style_policy(LeftTab::Jobs, false),
            Some(StyleId::TreeItemDisabled)
        );
        assert_eq!(job_row_style_policy(LeftTab::Jobs, true), None);
    }

    #[test]
    fn triage_tabs_never_apply_disabled_style() {
        assert_eq!(job_row_style_policy(LeftTab::TriageReview, false), None);
        assert_eq!(job_row_style_policy(LeftTab::TriageReview, true), None);
        assert_eq!(job_row_style_policy(LeftTab::TriageResults, false), None);
        assert_eq!(job_row_style_policy(LeftTab::TriageResults, true), None);
    }

    // ── Per-tab check policy tests ────────────────────────────────────────────

    #[test]
    fn check_policy_only_active_in_triage_review_during_interactive_phase() {
        let mut job = make_job(1, "https://x.com/", Stage::Done, None, None, None);
        job.filter_status = Some(JobFilterStatus::AutoIncluded);

        // TriageReview + interactive → Checked
        assert_eq!(
            job_row_check_policy(LeftTab::TriageReview, true, &job),
            CheckState::Checked
        );
        // TriageReview + not interactive → Unchecked
        assert_eq!(
            job_row_check_policy(LeftTab::TriageReview, false, &job),
            CheckState::Hidden
        );
        // Jobs tab → always Hidden
        assert_eq!(
            job_row_check_policy(LeftTab::Jobs, true, &job),
            CheckState::Hidden
        );
        // TriageResults → always Hidden
        assert_eq!(
            job_row_check_policy(LeftTab::TriageResults, true, &job),
            CheckState::Hidden
        );
    }

    #[test]
    fn triage_results_use_hidden_state_lane_for_markers() {
        let mut job = make_job(1, "https://example.com", Stage::Done, None, None, None);
        job.has_summary = true;
        job.summary_title = Some("Example headline".to_string());
        job.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "Business".to_string(),
            tags: vec!["capex".to_string()],
        });
        job.links = vec![make_link_row(
            0,
            "Related link",
            LinkDownloadState::Downloaded {
                path: "cache/article.html".into(),
            },
        )];
        job.link_count = job.links.len();
        let mut view = make_view(vec![job]);
        view.left_pane.left_tab = LeftTab::TriageResults;

        let items = build_job_tree(&view);
        assert_eq!(items[0].state, CheckState::Hidden);
        assert_eq!(items[0].children[0].state, CheckState::Hidden);
    }

    #[test]
    fn render_enables_open_browser_when_selected_url_is_some() {
        init_logging();
        let mut view = make_view(vec![]);
        view.selected_url = Some("https://example.com".to_string());
        let mut tree_state = TreeRenderState::new();
        let window_id = WindowId::new(1);
        let cmds = render(window_id, &view, &mut tree_state);
        let enabled = cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
                if *control_id == BUTTON_OPEN_BROWSER
            )
        });
        assert!(enabled, "BUTTON_OPEN_BROWSER should be enabled");
    }

    #[test]
    fn render_disables_open_browser_when_selected_url_is_none() {
        init_logging();
        let view = make_view(vec![]);
        let mut tree_state = TreeRenderState::new();
        let window_id = WindowId::new(1);
        let cmds = render(window_id, &view, &mut tree_state);
        let disabled = cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BUTTON_OPEN_BROWSER
            )
        });
        assert!(disabled, "BUTTON_OPEN_BROWSER should be disabled");
    }

    #[test]
    fn stop_button_uses_secondary_style_when_not_running() {
        init_logging();
        let view = make_view(vec![]);
        let mut tree_state = TreeRenderState::new();
        let window_id = WindowId::new(1);
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::ApplyStyleToControl { control_id, style_id, .. }
                if *control_id == BUTTON_STOP && *style_id == StyleId::SecondaryButton
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BUTTON_STOP
            )
        }));
    }

    #[test]
    fn stop_button_uses_destructive_style_when_running() {
        init_logging();
        let mut view = make_view(vec![]);
        view.session = SessionState::Running;
        let mut tree_state = TreeRenderState::new();
        let window_id = WindowId::new(1);
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::ApplyStyleToControl { control_id, style_id, .. }
                if *control_id == BUTTON_STOP && *style_id == StyleId::DestructiveButton
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
                if *control_id == BUTTON_STOP
            )
        }));
    }

    #[test]
    fn render_is_idempotent_for_open_browser_state() {
        init_logging();
        let view = make_view(vec![]);
        let mut tree_state = TreeRenderState::new();
        let window_id = WindowId::new(1);
        // First render sets initial state
        render(window_id, &view, &mut tree_state);
        // Second render should not emit SetControlEnabled for BUTTON_OPEN_BROWSER
        let cmds = render(window_id, &view, &mut tree_state);
        let changed = cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, .. }
                if *control_id == BUTTON_OPEN_BROWSER
            )
        });
        assert!(
            !changed,
            "BUTTON_OPEN_BROWSER state should not change on second render"
        );
    }

    #[test]
    fn run_button_disabled_when_can_run_false() {
        let window_id = WindowId::new(10);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.can_run = false;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BTN_PROMPT_LAB_RUN
            )
        }));
    }

    #[test]
    fn template_apply_button_disabled_when_validation_errors_exist() {
        let window_id = WindowId::new(15);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.template_editor_open = true;
        view.left_pane.prompt_lab.template_dirty = true;
        view.left_pane.prompt_lab.template_validation_errors = vec!["unknown var".to_string()];
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BTN_PROMPT_LAB_TEMPLATE_APPLY
            )
        }));
    }

    #[test]
    fn template_save_button_disabled_when_not_applied() {
        let window_id = WindowId::new(16);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.template_editor_open = true;
        view.left_pane.prompt_lab.template_applied = false;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BTN_PROMPT_LAB_TEMPLATE_SAVE
            )
        }));
    }

    #[test]
    fn template_errors_are_rendered_in_template_status_label() {
        let window_id = WindowId::new(17);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.template_editor_open = true;
        view.left_pane.prompt_lab.template_validation_errors = vec![
            "first validation problem".to_string(),
            "second validation problem".to_string(),
        ];
        let cmds = render(window_id, &view, &mut tree_state);
        let status = control_text(&cmds, LABEL_PROMPT_LAB_TEMPLATE_STATUS)
            .expect("template status text rendered");
        assert!(status.contains("first validation problem"));
        assert!(status.contains("second validation problem"));
    }

    #[test]
    fn compare_start_disabled_by_default() {
        let window_id = WindowId::new(18);
        let mut tree_state = TreeRenderState::new();
        let view = make_view(vec![]);
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BTN_COMPARE_START
            )
        }));
    }

    #[test]
    fn compare_start_enabled_when_two_draft_candidates() {
        let window_id = WindowId::new(19);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.draft_candidates = vec![
            harvester_core::PromptLabCompareCandidateView {
                candidate_id: 1,
                label: "c1".to_string(),
                stage_label: "Triage".to_string(),
                model_label: "default".to_string(),
                prompt_version_label: "active".to_string(),
                has_context_override: false,
                has_template_override: false,
            },
            harvester_core::PromptLabCompareCandidateView {
                candidate_id: 2,
                label: "c2".to_string(),
                stage_label: "Triage".to_string(),
                model_label: "default".to_string(),
                prompt_version_label: "active".to_string(),
                has_context_override: false,
                has_template_override: false,
            },
        ];
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
                if *control_id == BTN_COMPARE_START
            )
        }));
    }

    #[test]
    fn prompt_lab_section_checkbox_states_reflect_view_state() {
        let window_id = WindowId::new(33);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.compare_section_open = true;
        view.left_pane.prompt_lab.context_section_open = false;
        view.left_pane.prompt_lab.template_section_open = true;
        view.left_pane.prompt_lab.run_details_section_open = true;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetCheckBoxChecked { control_id, checked: true, .. }
                if *control_id == CHK_PROMPT_LAB_SECTION_COMPARE
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetCheckBoxChecked { control_id, checked: false, .. }
                if *control_id == CHK_PROMPT_LAB_SECTION_CONTEXT
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetCheckBoxChecked { control_id, checked: true, .. }
                if *control_id == CHK_PROMPT_LAB_SECTION_RUN_DETAILS
            )
        }));
    }

    #[test]
    fn jobs_scope_toggle_reflects_scope_state() {
        let window_id = WindowId::new(39);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.job_list_scope = JobListScope::SinceCheckpoint;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetToggleSwitchState { control_id, checked: true, .. }
                if *control_id == TS_JOBS_SCOPE
            )
        }));

        view.left_pane.job_list_scope = JobListScope::All;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetToggleSwitchState { control_id, checked: false, .. }
                if *control_id == TS_JOBS_SCOPE
            )
        }));
    }

    #[test]
    fn token_progress_uses_since_checkpoint_scope_total_when_enabled() {
        let window_id = WindowId::new(41);
        let mut tree_state = TreeRenderState::new();
        let mut since_job = make_job(
            1,
            "https://since.example",
            Stage::Done,
            Some(JobResultKind::Success),
            Some(50),
            None,
        );
        since_job.is_since_checkpoint = true;
        let all_time_job = make_job(
            2,
            "https://all.example",
            Stage::Done,
            Some(JobResultKind::Success),
            Some(150),
            None,
        );
        let mut view = make_view(vec![since_job, all_time_job]);
        view.total_tokens = 200;
        view.token_limit = 200_000;
        view.left_pane.job_list_scope = JobListScope::SinceCheckpoint;

        let cmds = render(window_id, &view, &mut tree_state);

        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlText { control_id, text, .. }
                if *control_id == LABEL_TOKEN_PROGRESS
                    && text == "50 / 200K"
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetProgressBarPosition { control_id, position, .. }
                if *control_id == PROGRESS_TOKENS && *position == 50
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::ApplyStyleToControl { control_id, style_id, .. }
                if *control_id == PROGRESS_TOKENS && *style_id == StyleId::StatusMeter
            )
        }));
    }

    #[test]
    fn token_progress_stays_muted_below_limit_even_when_high() {
        let window_id = WindowId::new(42);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(Vec::new());
        view.total_tokens = 97_002;
        view.token_limit = 100_000;

        let cmds = render(window_id, &view, &mut tree_state);

        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::ApplyStyleToControl { control_id, style_id, .. }
                if *control_id == PROGRESS_TOKENS && *style_id == StyleId::StatusMeter
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlText { control_id, text, .. }
                if *control_id == LABEL_TOKEN_PROGRESS
                    && text == "97K / 100K"
            )
        }));
    }

    #[test]
    fn token_progress_escalates_to_accent_at_limit() {
        let window_id = WindowId::new(43);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(Vec::new());
        view.total_tokens = 100_000;
        view.token_limit = 100_000;

        let cmds = render(window_id, &view, &mut tree_state);

        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::ApplyStyleToControl { control_id, style_id, .. }
                if *control_id == PROGRESS_TOKENS && *style_id == StyleId::ProgressBar
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlText { control_id, text, .. }
                if *control_id == LABEL_TOKEN_PROGRESS
                    && text == "100K / 100K"
            )
        }));
    }

    #[test]
    fn left_header_only_renders_meta_row() {
        let window_id = WindowId::new(40);
        let mut tree_state = TreeRenderState::new();
        let empty_view = make_view(vec![JobRowView {
            job_id: 1,
            url: "https://example.com".to_string(),
            stage: Stage::Done,
            outcome: Some(JobResultKind::Success),
            tokens: None,
            bytes: None,
            link_count: 0,
            downloaded_link_count: 0,
            links: vec![],
            triage_annotation: None,
            has_summary: false,
            summary_title: None,
            filter_status: None,
            has_analysis: false,
            origin: JobOrigin::Direct,
            is_since_checkpoint: true,
        }]);
        let mut empty_view = empty_view;
        empty_view.left_pane.left_tab = LeftTab::TriageResults;
        empty_view.left_pane_header = LeftPaneHeaderView {
            title: "Triage Results".to_string(),
            scope_label: None,
            count_label: Some("no triage results yet".to_string()),
            state_label: None,
        };
        let empty_cmds = render(window_id, &empty_view, &mut tree_state);
        let empty_meta =
            control_text(&empty_cmds, LABEL_JOBS_HEADER_META).expect("empty triage meta rendered");
        assert_eq!(empty_meta, "no triage results yet");
        assert!(control_text(&empty_cmds, LABEL_JOBS_HEADER_TITLE).is_none());

        let mut populated_view = empty_view.clone();
        populated_view.jobs[0].triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 1,
            category: "keep".to_string(),
            tags: vec![],
        });
        populated_view.left_pane_header = LeftPaneHeaderView {
            title: "Triage Results".to_string(),
            scope_label: None,
            count_label: Some("1 with triage".to_string()),
            state_label: None,
        };
        let populated_cmds = render(window_id, &populated_view, &mut tree_state);
        let populated_meta = control_text(&populated_cmds, LABEL_JOBS_HEADER_META)
            .expect("populated triage meta rendered");
        assert_eq!(populated_meta, "1 with triage");
        assert!(control_text(&populated_cmds, LABEL_JOBS_HEADER_TITLE).is_none());
    }

    #[test]
    fn status_bar_uses_warning_severity_for_ai_unavailable_message() {
        let window_id = WindowId::new(41);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.ai_unavailable_message =
            Some("AI features unavailable: OPENAI_API_KEY is not set".to_string());

        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::UpdateLabelText { control_id, severity: MessageSeverity::Warning, text, .. }
                if *control_id == LABEL_STATUS && text.contains("OPENAI_API_KEY is not set")
            )
        }));
    }

    #[test]
    fn triage_results_meta_uses_ai_unavailable_copy_when_blocked() {
        let window_id = WindowId::new(42);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.left_tab = LeftTab::TriageResults;
        view.ai_unavailable_message =
            Some("AI features unavailable: OPENAI_API_KEY is not set".to_string());
        view.left_pane_header = LeftPaneHeaderView {
            title: "Triage Results".to_string(),
            scope_label: None,
            count_label: Some("no triage results yet".to_string()),
            state_label: Some("AI unavailable".to_string()),
        };

        let cmds = render(window_id, &view, &mut tree_state);
        let meta = control_text(&cmds, LABEL_JOBS_HEADER_META).expect("triage meta rendered");
        assert_eq!(meta, "no triage results yet · AI unavailable");
        assert!(control_text(&cmds, LABEL_JOBS_HEADER_TITLE).is_none());
    }

    #[test]
    fn triage_results_meta_preserves_count_when_ai_unavailable() {
        let window_id = WindowId::new(43);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![JobRowView {
            job_id: 1,
            url: "https://example.com".to_string(),
            stage: Stage::Done,
            outcome: Some(JobResultKind::Success),
            tokens: None,
            bytes: None,
            link_count: 0,
            downloaded_link_count: 0,
            links: vec![],
            triage_annotation: Some(harvester_core::TriageAnnotationView {
                priority: 1,
                category: "keep".to_string(),
                tags: vec![],
            }),
            has_summary: false,
            summary_title: None,
            filter_status: None,
            has_analysis: true,
            origin: JobOrigin::Direct,
            is_since_checkpoint: true,
        }]);
        view.left_pane.left_tab = LeftTab::TriageResults;
        view.ai_unavailable_message =
            Some("AI features unavailable: OPENAI_API_KEY is not set".to_string());
        view.left_pane_header = LeftPaneHeaderView {
            title: "Triage Results".to_string(),
            scope_label: None,
            count_label: Some("1 with triage".to_string()),
            state_label: Some("AI unavailable".to_string()),
        };

        let cmds = render(window_id, &view, &mut tree_state);
        let meta = control_text(&cmds, LABEL_JOBS_HEADER_META).expect("triage meta rendered");
        assert_eq!(meta, "1 with triage · AI unavailable");
        assert!(control_text(&cmds, LABEL_JOBS_HEADER_TITLE).is_none());
    }

    #[test]
    fn prompt_lab_mode_and_stage_radio_states_reflect_view_state() {
        let window_id = WindowId::new(38);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.advanced_mode = true;
        view.left_pane.prompt_lab.selected_stage = PromptLabStage::Summary;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetRadioButtonChecked { control_id, checked: true, .. }
                if *control_id == BTN_PROMPT_LAB_MODE_ADVANCED
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetRadioButtonChecked { control_id, checked: false, .. }
                if *control_id == BTN_PROMPT_LAB_MODE_BASIC
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetRadioButtonChecked { control_id, checked: true, .. }
                if *control_id == BTN_STAGE_SUMMARY
            )
        }));
    }

    #[test]
    fn prompt_lab_tab_advanced_layout_does_not_depend_on_legacy_visible_flag() {
        let window_id = WindowId::new(39);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.left_tab = LeftTab::PromptLab;
        view.left_pane.prompt_lab.visible = false;
        view.left_pane.prompt_lab.advanced_mode = true;

        let cmds = render(window_id, &view, &mut tree_state);
        let rules = cmds
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::DefineLayout { rules, .. } => Some(rules),
                _ => None,
            })
            .expect("DefineLayout emitted");

        let compare_header_size = rules
            .iter()
            .find(|rule| rule.control_id == PANEL_PROMPT_LAB_COMPARE_HEADER_ROW)
            .and_then(|rule| rule.fixed_size)
            .expect("compare header fixed size");
        assert_ne!(compare_header_size, 0);
    }

    #[test]
    fn prompt_lab_model_selector_defaults_to_index_zero_on_first_render() {
        let window_id = WindowId::new(34);
        let mut tree_state = TreeRenderState::new();
        let view = make_view(vec![]);

        let cmds = render(window_id, &view, &mut tree_state);

        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetComboBoxSelection {
                    control_id,
                    selected_index: Some(0),
                    ..
                } if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR
            )
        }));
    }

    #[test]
    fn prompt_lab_model_selector_emits_catalog_items() {
        let window_id = WindowId::new(35);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.model_catalog = vec![
            ModelId::new(
                harvester_engine::llm::ProviderKind::OpenAi,
                harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI,
            ),
            ModelId::new(harvester_engine::llm::ProviderKind::OpenAi, "o3-mini"),
        ];

        let cmds = render(window_id, &view, &mut tree_state);

        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetComboBoxItems {
                    control_id,
                    items,
                    ..
                } if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR
                    && items == &vec![
                        "Default".to_string(),
                        harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI.to_string(),
                        "o3-mini".to_string(),
                    ]
            )
        }));
    }

    #[test]
    fn prompt_lab_model_selector_replays_on_hidden_to_visible_transition() {
        let window_id = WindowId::new(36);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.left_tab = LeftTab::PromptLab;
        view.left_pane.prompt_lab.model_catalog = vec![ModelId::new(
            harvester_engine::llm::ProviderKind::OpenAi,
            harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI,
        )];

        let initial_cmds = render(window_id, &view, &mut tree_state);
        assert!(initial_cmds.iter().any(|cmd| {
            matches!(cmd, PlatformCommand::SetComboBoxItems { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
        }));
        assert!(initial_cmds.iter().any(|cmd| {
            matches!(cmd, PlatformCommand::SetComboBoxSelection { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
        }));

        view.left_pane.left_tab = LeftTab::Jobs;
        let _ = render(window_id, &view, &mut tree_state);

        view.left_pane.left_tab = LeftTab::PromptLab;
        let reopen_cmds = render(window_id, &view, &mut tree_state);
        assert!(reopen_cmds.iter().any(|cmd| {
            matches!(cmd, PlatformCommand::SetComboBoxItems { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
        }));
        assert!(reopen_cmds.iter().any(|cmd| {
            matches!(cmd, PlatformCommand::SetComboBoxSelection { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
        }));
    }

    #[test]
    fn prompt_lab_model_selector_unchanged_visible_state_is_idempotent() {
        let window_id = WindowId::new(37);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.visible = true;
        view.left_pane.prompt_lab.model_catalog = vec![ModelId::new(
            harvester_engine::llm::ProviderKind::OpenAi,
            harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI,
        )];

        let _ = render(window_id, &view, &mut tree_state);
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(!cmds.iter().any(|cmd| {
            matches!(cmd, PlatformCommand::SetComboBoxItems { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
        }));
        assert!(!cmds.iter().any(|cmd| {
            matches!(cmd, PlatformCommand::SetComboBoxSelection { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
        }));
    }

    #[test]
    fn resolve_button_enabled_when_not_pending() {
        let window_id = WindowId::new(12);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.resolve_pending = true;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BTN_PROMPT_LAB_RESOLVE
            )
        }));

        view.left_pane.prompt_lab.resolve_pending = false;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
                if *control_id == BTN_PROMPT_LAB_RESOLVE
            )
        }));
    }

    /// The Prompt Lab now lives in its own tab; a completed run does NOT override VIEWER_PREVIEW.
    #[test]
    fn prompt_lab_run_does_not_override_summary_viewer() {
        let window_id = WindowId::new(13);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab = completed_prompt_lab_view();
        let cmds = render(window_id, &view, &mut tree_state);
        // The Prompt Lab output JSON should NOT appear in VIEWER_PREVIEW.
        assert!(!cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetRichEditContent { control_id, rtf_text, .. }
                if *control_id == VIEWER_PREVIEW && rtf_text.contains("ok")
            )
        }));
        // LABEL_PREVIEW_HEADER should not say "Prompt Lab".
        assert!(!cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlText { control_id, text, .. }
                if *control_id == LABEL_PREVIEW_HEADER && text.contains("Prompt Lab")
            )
        }));
    }

    #[test]
    fn summary_tab_without_selected_article_shows_empty_state_not_briefing_preview() {
        let window_id = WindowId::new(40);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.right_pane.active_tab = AppTab::Summary;
        view.right_pane.summary_markdown = None;
        view.preview_text = Some("Briefing content should not leak".to_string());
        view.right_pane.briefing_markdown = Some("Briefing content should not leak".to_string());

        let cmds = render(window_id, &view, &mut tree_state);

        assert_eq!(
            tree_state.prev_preview_text.as_deref(),
            Some(SUMMARY_EMPTY_STATE_MARKDOWN)
        );
        assert!(!cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetRichEditContent { control_id, rtf_text, .. }
                if *control_id == VIEWER_PREVIEW && rtf_text.contains("Briefing content should not leak")
            )
        }));
    }

    #[test]
    fn render_idempotent_on_unchanged_prompt_lab_view() {
        let window_id = WindowId::new(14);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab = completed_prompt_lab_view();
        let _ = render(window_id, &view, &mut tree_state);
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(!cmds
            .iter()
            .any(|cmd| matches!(cmd, PlatformCommand::SetRichEditContent { .. })));
    }

    fn control_text(
        cmds: &[PlatformCommand],
        control_id: commanductui::types::ControlId,
    ) -> Option<&str> {
        cmds.iter().find_map(|cmd| match cmd {
            PlatformCommand::SetControlText {
                control_id: rendered_id,
                text,
                ..
            } if *rendered_id == control_id => Some(text.as_str()),
            _ => None,
        })
    }

    #[test]
    fn status_bar_omits_llm_usage_when_empty() {
        assert_eq!(format_llm_usage_status(&[]), None);
    }

    #[test]
    fn status_bar_includes_llm_usage_segment() {
        let rows = vec![LlmModelUsageView {
            model: "alpha".to_string(),
            input_tokens: 12_000,
            output_tokens: 3_000,
        }];
        let result = format_llm_usage_status(&rows).expect("Some");
        assert!(result.contains("alpha"));
        assert!(result.contains("in=12K"));
        assert!(result.contains("out=3K"));
        assert!(!result.contains('+'));
    }

    #[test]
    fn status_bar_collapses_when_model_count_exceeds_limit() {
        let rows = vec![
            LlmModelUsageView {
                model: "alpha".to_string(),
                input_tokens: 1_000,
                output_tokens: 500,
            },
            LlmModelUsageView {
                model: "beta".to_string(),
                input_tokens: 2_000,
                output_tokens: 1_000,
            },
            LlmModelUsageView {
                model: "gamma".to_string(),
                input_tokens: 3_000,
                output_tokens: 1_500,
            },
        ];
        let result = format_llm_usage_status(&rows).expect("Some");
        assert!(result.contains("alpha: in=1K out=500"));
        assert!(result.contains("beta: in=2K out=1K"));
        assert!(result.contains("+1 models"));
        assert!(!result.contains("gamma"));
    }

    #[test]
    fn trends_chart_data_emits_set_chart_data() {
        use harvester_core::{CategoryTrendView, EntityLineView, TrendCategory, TrendsTabView};
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let mut view = AppViewModel::default();
        view.right_pane.trends = TrendsTabView {
            is_loading: false,
            active_category: TrendCategory::Companies,
            category_data: Some(CategoryTrendView {
                weeks: vec!["W1".to_string(), "W2".to_string(), "W3".to_string()],
                lines: vec![
                    EntityLineView {
                        label: "Acme".to_string(),
                        weekly_counts: vec![1, 2, 3],
                        total_count: 6,
                    },
                    EntityLineView {
                        label: "Beta".to_string(),
                        weekly_counts: vec![3, 2, 1],
                        total_count: 6,
                    },
                ],
                total_entity_count: 2,
            }),
        };
        let cmds = render(window_id, &view, &mut state);
        let chart_cmd = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        });
        assert!(
            chart_cmd.is_some(),
            "SetChartData not emitted for CHART_TRENDS"
        );
        if let Some(PlatformCommand::SetChartData { data, .. }) = chart_cmd {
            assert_eq!(data.lines.len(), 2);
            assert_eq!(data.lines[0].label, "Acme");
            assert_eq!(data.week_labels, vec!["W1", "W2", "W3"]);
            assert!(!data.is_loading);
        }
    }

    #[test]
    fn trends_chart_loading_state_emits_empty_packet() {
        use harvester_core::TrendsTabView;
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let mut view = AppViewModel::default();
        view.right_pane.trends = TrendsTabView {
            is_loading: true,
            ..Default::default()
        };
        let cmds = render(window_id, &view, &mut state);
        let chart_cmd = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        });
        assert!(
            chart_cmd.is_some(),
            "SetChartData not emitted during loading"
        );
        if let Some(PlatformCommand::SetChartData { data, .. }) = chart_cmd {
            assert!(data.is_loading);
            assert!(data.lines.is_empty());
        }
    }

    #[test]
    fn trends_chart_data_truncates_to_five_lines() {
        use harvester_core::{CategoryTrendView, EntityLineView, TrendCategory, TrendsTabView};
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let mut view = AppViewModel::default();
        let lines: Vec<EntityLineView> = (0..10)
            .map(|i| EntityLineView {
                label: format!("Entity{i}"),
                weekly_counts: vec![i as u32, i as u32 + 1],
                total_count: (2 * i) as u32,
            })
            .collect();
        view.right_pane.trends = TrendsTabView {
            is_loading: false,
            active_category: TrendCategory::Companies,
            category_data: Some(CategoryTrendView {
                weeks: vec!["W1".to_string(), "W2".to_string()],
                lines,
                total_entity_count: 10,
            }),
        };
        let cmds = render(window_id, &view, &mut state);
        let chart_cmd = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        });
        assert!(chart_cmd.is_some(), "SetChartData not emitted");
        if let Some(PlatformCommand::SetChartData { data, .. }) = chart_cmd {
            assert_eq!(data.lines.len(), 5, "expected at most 5 lines from take(5)");
        }
    }

    fn make_five_line_trends_view() -> AppViewModel {
        use harvester_core::{CategoryTrendView, EntityLineView, TrendCategory, TrendsTabView};
        let mut view = AppViewModel::default();
        let lines: Vec<EntityLineView> = (0..5)
            .map(|i| EntityLineView {
                label: format!("Entity{i}"),
                weekly_counts: vec![i as u32, i as u32 + 1],
                total_count: (2 * i) as u32,
            })
            .collect();
        view.right_pane.trends = TrendsTabView {
            is_loading: false,
            active_category: TrendCategory::Companies,
            category_data: Some(CategoryTrendView {
                weeks: vec!["W1".to_string(), "W2".to_string()],
                lines,
                total_entity_count: 5,
            }),
        };
        view
    }

    #[test]
    fn trends_top_two_lines_are_primary_emphasis() {
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let view = make_five_line_trends_view();
        let cmds = render(window_id, &view, &mut state);
        if let Some(PlatformCommand::SetChartData { data, .. }) = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        }) {
            assert!(
                matches!(data.lines[0].emphasis, ChartLineEmphasis::Primary),
                "line 0 should be Primary"
            );
            assert!(
                matches!(data.lines[1].emphasis, ChartLineEmphasis::Primary),
                "line 1 should be Primary"
            );
        } else {
            panic!("SetChartData not emitted");
        }
    }

    #[test]
    fn trends_lines_2_to_4_are_secondary_emphasis() {
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let view = make_five_line_trends_view();
        let cmds = render(window_id, &view, &mut state);
        if let Some(PlatformCommand::SetChartData { data, .. }) = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        }) {
            for i in 2..5 {
                assert!(
                    matches!(data.lines[i].emphasis, ChartLineEmphasis::Secondary),
                    "line {i} should be Secondary"
                );
            }
        } else {
            panic!("SetChartData not emitted");
        }
    }

    #[test]
    fn trends_all_lines_have_end_label() {
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let view = make_five_line_trends_view();
        let cmds = render(window_id, &view, &mut state);
        if let Some(PlatformCommand::SetChartData { data, .. }) = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        }) {
            for (i, line) in data.lines.iter().enumerate() {
                assert!(
                    line.end_label.is_some(),
                    "line {i} should have end_label set"
                );
            }
        } else {
            panic!("SetChartData not emitted");
        }
    }

    #[test]
    fn trends_show_end_labels_is_true() {
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let view = make_five_line_trends_view();
        let cmds = render(window_id, &view, &mut state);
        if let Some(PlatformCommand::SetChartData { data, .. }) = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        }) {
            assert!(data.show_end_labels, "show_end_labels should be true");
        } else {
            panic!("SetChartData not emitted");
        }
    }