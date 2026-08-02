use super::*;

fn list_box_command_count(commands: &[PlatformCommand]) -> usize {
    commands
        .iter()
        .filter(|command| {
            matches!(
                command,
                PlatformCommand::SetListBoxRowDensity { .. }
                    | PlatformCommand::PopulateListBox { .. }
                    | PlatformCommand::SetListBoxSelection { .. }
            )
        })
        .count()
}


#[test]
fn list_box_initial_render_emits_density_and_population() {
    let window_id = WindowId::new(67);
    let mut tree_state = TreeRenderState::new();
    let view = make_view(vec![make_job(
        1,
        "https://example.com",
        Stage::Queued,
        None,
        None,
        None,
    )]);

    let commands = render(window_id, &view, &mut tree_state);

    assert!(commands
        .iter()
        .any(|command| matches!(command, PlatformCommand::SetListBoxRowDensity { .. })));
    assert!(commands
        .iter()
        .any(|command| matches!(command, PlatformCommand::PopulateListBox { .. })));
}


#[test]
fn list_box_identical_second_render_emits_no_commands() {
    let window_id = WindowId::new(68);
    let mut tree_state = TreeRenderState::new();
    let view = make_view(vec![make_job(
        1,
        "https://example.com",
        Stage::Queued,
        None,
        None,
        None,
    )]);

    let _ = render(window_id, &view, &mut tree_state);
    let commands = render(window_id, &view, &mut tree_state);

    assert_eq!(list_box_command_count(&commands), 0);
}


#[test]
fn list_box_selection_only_change_emits_selection_without_population() {
    let window_id = WindowId::new(69);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![make_job(
        1,
        "https://example.com",
        Stage::Queued,
        None,
        None,
        None,
    )]);
    let _ = render(window_id, &view, &mut tree_state);

    view.selected_job_id = Some(1);
    let commands = render(window_id, &view, &mut tree_state);

    assert_eq!(list_box_command_count(&commands), 1);
    assert!(commands.iter().any(|command| {
        matches!(
            command,
            PlatformCommand::SetListBoxSelection { item_id, .. }
                if *item_id == ListBoxItemId::new(1)
        )
    }));
}


#[test]
fn list_box_badge_or_content_change_emits_only_population() {
    let window_id = WindowId::new(70);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![make_job(
        1,
        "https://example.com",
        Stage::Queued,
        None,
        None,
        None,
    )]);
    let _ = render(window_id, &view, &mut tree_state);

    view.jobs[0].stage = Stage::Downloading;
    let commands = render(window_id, &view, &mut tree_state);

    assert_eq!(list_box_command_count(&commands), 1);
    assert!(commands
        .iter()
        .any(|command| matches!(command, PlatformCommand::PopulateListBox { .. })));
}


#[test]
fn list_box_tab_transition_emits_changed_density() {
    let window_id = WindowId::new(71);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![make_job(
        1,
        "https://example.com",
        Stage::Done,
        Some(JobResultKind::Success),
        None,
        None,
    )]);
    let _ = render(window_id, &view, &mut tree_state);

    view.left_pane.left_tab = LeftTab::TriageResults;
    let commands = render(window_id, &view, &mut tree_state);

    assert!(commands.iter().any(|command| {
        matches!(
            command,
            PlatformCommand::SetListBoxRowDensity { density, .. }
                if *density == ListBoxRowDensity::Compact
        )
    }));
}


#[test]
fn list_box_selection_some_to_none_with_unchanged_items_emits_no_command() {
    let window_id = WindowId::new(72);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![make_job(
        1,
        "https://example.com",
        Stage::Queued,
        None,
        None,
        None,
    )]);
    view.selected_job_id = Some(1);
    let _ = render(window_id, &view, &mut tree_state);

    view.selected_job_id = None;
    let commands = render(window_id, &view, &mut tree_state);

    assert_eq!(list_box_command_count(&commands), 0);
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
    assert_eq!(item.metadata, "Security");
}


#[test]
fn triage_results_priority_badge_maps_full_scale() {
    // The triage prompt returns priority 1 (lowest) to 5 (highest/most urgent).
    // See crates/harvester_engine/src/llm/prompts/triage.rs.
    let cases: &[(u8, StyleId)] = &[
        (1, StyleId::BadgePriorityLow),
        (2, StyleId::BadgePriorityLow),
        (3, StyleId::BadgePriorityMedium),
        (4, StyleId::BadgePriorityHigh),
        (5, StyleId::BadgePriorityCritical),
    ];

    for (priority, expected_style) in cases {
        let mut job = make_job(1, "https://example.com", Stage::Done, None, None, None);
        job.summary_title = Some("Example headline".to_string());
        job.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: *priority,
            category: "business".to_string(),
            tags: vec!["tag-a".to_string()],
        });

        let item = build_list_box_item(LeftTab::TriageResults, &job);

        assert_eq!(item.badges.len(), 2, "priority {priority}");
        assert_eq!(item.badges[0].text, format!("P{priority}"));
        assert_eq!(
            item.badges[0].style, *expected_style,
            "priority {priority} should map to {:?}",
            expected_style
        );
        assert_eq!(item.badges[1].text, "Business");
        assert_eq!(item.badges[1].style, StyleId::BadgeCategory);
        assert_eq!(item.metadata, "");
    }
}


#[test]
fn triage_results_request_compact_list_density() {
    init_logging();
    let window_id = WindowId::new(66);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![make_job(
        1,
        "https://example.com",
        Stage::Done,
        Some(JobResultKind::Success),
        None,
        None,
    )]);
    view.left_pane.left_tab = LeftTab::TriageResults;

    let commands = render(window_id, &view, &mut tree_state);

    assert!(commands.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetListBoxRowDensity { density, .. }
                if *density == ListBoxRowDensity::Compact
        )
    }));
}


// ── Scope filter tests ────────────────────────────────────────────────────

#[test]
fn list_box_items_for_jobs_tab_follow_visible_jobs_after_filter() {
    init_logging();

    let mut job_visible = make_job(
        1,
        "https://a.com/",
        Stage::Done,
        Some(JobResultKind::Success),
        None,
        None,
    );
    job_visible.has_summary = true;
    job_visible.summary_title = Some("Visible".to_string());

    let mut job_hidden = make_job(
        2,
        "https://b.com/",
        Stage::Done,
        Some(JobResultKind::Success),
        None,
        None,
    );
    job_hidden.has_summary = true;
    job_hidden.summary_title = Some("Hidden".to_string());

    let mut view = make_view(vec![job_visible, job_hidden]);
    view.left_pane.left_tab = LeftTab::Jobs;
    view.left_pane.visible_jobs_after_filter = vec![1];

    let populated = build_list_box_items(&view);

    assert_eq!(populated.len(), 1);
    assert_eq!(populated[0].id, ListBoxItemId::new(1));
    assert!(populated[0].title.contains("Visible"));
}


#[test]
fn list_box_selected_id_omitted_when_selection_filtered_out() {
    init_logging();
    let window_id = WindowId::new(61);
    let mut tree_state = TreeRenderState::new();

    let mut job_in = make_job(
        1,
        "https://a.com/",
        Stage::Done,
        Some(JobResultKind::Success),
        None,
        None,
    );
    job_in.has_summary = true;
    job_in.summary_title = Some("Visible".to_string());

    let mut job_out = make_job(
        2,
        "https://b.com/",
        Stage::Done,
        Some(JobResultKind::Success),
        None,
        None,
    );
    job_out.has_summary = true;
    job_out.summary_title = Some("Filtered".to_string());

    let mut view = make_view(vec![job_in, job_out]);
    view.selected_job_id = Some(2);
    view.left_pane.left_tab = LeftTab::Jobs;
    view.left_pane.visible_jobs_after_filter = vec![1];

    let cmds = render(window_id, &view, &mut tree_state);

    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::PopulateListBox { items, .. }
                if items.iter().all(|item| item.id != ListBoxItemId::new(2))
        )
    }));
    assert!(!cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetListBoxSelection { control_id, item_id, .. }
                if *control_id == TREE_JOBS && *item_id == ListBoxItemId::new(2)
        )
    }));
}


#[test]
fn list_box_selection_is_reemitted_when_selected_item_leaves_and_returns() {
    init_logging();
    let window_id = WindowId::new(62);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![
        make_job(1, "https://a.com/", Stage::Done, None, None, None),
        make_job(2, "https://b.com/", Stage::Done, None, None, None),
    ]);
    view.selected_job_id = Some(2);
    view.left_pane.left_tab = LeftTab::Jobs;

    let initial_commands = render(window_id, &view, &mut tree_state);
    assert!(initial_commands.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetListBoxSelection { control_id, item_id, .. }
                if *control_id == TREE_JOBS && *item_id == ListBoxItemId::new(2)
        )
    }));

    view.left_pane.visible_jobs_after_filter = vec![1];
    let filtered_commands = render(window_id, &view, &mut tree_state);
    assert!(filtered_commands
        .iter()
        .any(|cmd| matches!(cmd, PlatformCommand::PopulateListBox { .. })));
    assert!(!filtered_commands
        .iter()
        .any(|cmd| matches!(cmd, PlatformCommand::SetListBoxSelection { .. })));

    view.left_pane.visible_jobs_after_filter = vec![1, 2];
    let restored_commands = render(window_id, &view, &mut tree_state);
    assert!(restored_commands.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetListBoxSelection { control_id, item_id, .. }
                if *control_id == TREE_JOBS && *item_id == ListBoxItemId::new(2)
        )
    }));
}


#[test]
fn list_box_items_for_non_jobs_tabs_ignore_visible_jobs_after_filter() {
    init_logging();

    for tab in [
        LeftTab::TriageReview,
        LeftTab::TriageResults,
        LeftTab::PromptLab,
    ] {
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
        view.left_pane.left_tab = tab;
        view.left_pane.job_list_scope = JobListScope::SinceCheckpoint;
        view.left_pane.visible_jobs_after_filter = vec![2];

        let populated = build_list_box_items(&view);

        assert_eq!(
            populated.len(),
            1,
            "{tab:?} should still use its own scope filter"
        );
        assert_eq!(populated[0].id, ListBoxItemId::new(1));
    }
}


#[test]
fn results_tab_keeps_triage_rows_and_prepends_signal_outcome_badges() {
    init_logging();

    let mut job = make_job(
        1,
        "https://example.com/article",
        Stage::Done,
        Some(JobResultKind::Success),
        None,
        None,
    );
    job.summary_title = Some("Visible triage headline".to_string());
    job.triage_annotation = Some(harvester_core::TriageAnnotationView {
        priority: 5,
        category: "business".to_string(),
        tags: vec![],
    });
    let mut view = make_view(vec![job]);
    view.left_pane.left_tab = LeftTab::TriageResults;
    view.signal_candidate_rows = vec![SignalCandidateRow {
        job_id: 1,
        url: "https://example.com/article".to_string(),
        score: 80,
        score_band: ScoreBand::High,
        source_tier: harvester_engine::llm::dto::SourceTier::Tier1,
        themes: vec![],
        gist_truncated: "Signal gist".to_string(),
        dupes_count: 0,
        state_label: SignalCandidateRowState::Scored,
        signal_key: "k".to_string(),
        outcome: Some(SignalCandidateOutcome::Selected),
    }];

    let populated = build_list_box_items(&view);

    assert_eq!(populated.len(), 1);
    assert_eq!(populated[0].title, "Visible triage headline");
    assert_eq!(populated[0].badges[0].text, "ARCH");
    assert_eq!(populated[0].badges[1].text, "P5");
    assert_eq!(populated[0].badges[2].text, "Business");
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
    view.triage_results_reorder_suppressed = true;

    let populated = build_list_box_items(&view);

    assert_eq!(populated.len(), 2);
    assert!(
        populated[0].title.starts_with("low.com"),
        "first should stay job 1 while triage is running, got: {}",
        populated[0].title
    );
    assert!(populated[0].metadata.is_empty());
    assert!(
        populated[1].title.contains("High Priority"),
        "second should stay job 2 while triage is running, got: {}",
        populated[1].title
    );
}


#[test]
fn triage_results_tab_sorts_by_priority_during_download_progress() {
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
    view.operation_progress = Some(harvester_core::OperationProgress {
        label: "Downloading articles".to_string(),
        completed: 1,
        total: 2,
    });

    let populated = build_list_box_items(&view);

    assert_eq!(populated.len(), 2);
    assert_eq!(populated[0].id, ListBoxItemId::new(2));
    assert!(populated[0].title.contains("High Priority"));
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
    initial_view.triage_results_reorder_suppressed = true;
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
    updated_view.triage_results_reorder_suppressed = true;

    let cmds = render(window_id, &updated_view, &mut tree_state);

    let populated = cmds
        .iter()
        .find_map(|cmd| match cmd {
            PlatformCommand::PopulateListBox { items, .. } => Some(items),
            _ => None,
        })
        .expect("updated list emitted");
    assert_eq!(populated.len(), 2);
    assert_eq!(populated[0].id, ListBoxItemId::new(1));
    assert!(populated[0].title.contains("Now Highest"));
    assert!(populated[0].metadata.is_empty());
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

