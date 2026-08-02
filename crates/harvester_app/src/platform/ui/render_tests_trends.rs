use super::*;
use commanductui::ChartLineEmphasis;
use harvester_core::{CategoryTrendView, EntityLineView, TrendCategory, TrendsTabView};

#[test]
fn trends_chart_data_emits_set_chart_data() {
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
