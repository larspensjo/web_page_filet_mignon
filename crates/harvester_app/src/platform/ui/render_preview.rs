use commanductui::{ChartDataPacket, ChartLineData, ChartLineEmphasis};
use commanductui::{PlatformCommand, WindowId};
use engine_logging::engine_warn;
use harvester_core::{AppViewModel, TrendsTabView};

use super::constants::*;
use super::markdown_to_rtf::{convert_markdown_to_rtf, RTF_TRUNCATE_MARKER};
use super::render::{emit_if_changed, TreeRenderState};
use super::render_text::{strip_leading_h1, truncate_markdown_for_preview};

pub(super) const SUMMARY_EMPTY_STATE_MARKDOWN: &str =
    "No article selected\n\nSelect a job/article from the list to view its summary.";

fn format_preview_source_label(source_label: &str) -> String {
    let source_label = source_label.trim();
    if source_label.is_empty() {
        String::new()
    } else {
        format!("Source: {source_label}")
    }
}

pub(super) fn build_chart_data(trends: &TrendsTabView) -> ChartDataPacket {
    // COLORREF palette (0x00BBGGRR), warm-compatible chart colors.
    const COLORS: [u32; 10] = [
        0x004264C9, // #C96442 terracotta
        0x005777D9, // #D97757 coral
        0x00A5AEB0, // #B0AEA5 warm silver
        0x007F8687, // #87867F stone
        0x0059935E, // #5E9359 muted green
        0x005AACB8, // #B8AC5A warm gold
        0x008C6DAF, // #AF6D8C muted mauve
        0x0068A5C4, // #C4A568 sand
        0x00A0827A, // #7A82A0 cool-warm lavender
        0x006BB088, // #88B06B sage
    ];

    if trends.is_loading {
        return ChartDataPacket {
            lines: vec![],
            week_labels: vec![],
            is_loading: true,
            show_x_axis_labels: true,
            show_y_axis_labels: true,
            show_end_labels: false,
        };
    }
    let Some(cat_data) = &trends.category_data else {
        return ChartDataPacket {
            lines: vec![],
            week_labels: vec![],
            is_loading: false,
            show_x_axis_labels: true,
            show_y_axis_labels: true,
            show_end_labels: false,
        };
    };
    let lines = cat_data
        .lines
        .iter()
        .take(5)
        .enumerate()
        .map(|(i, el)| ChartLineData {
            label: el.label.clone(),
            weekly_counts: el.weekly_counts.clone(),
            color: COLORS[i],
            end_label: Some(el.label.clone()),
            emphasis: if i < 2 {
                ChartLineEmphasis::Primary
            } else {
                ChartLineEmphasis::Secondary
            },
        })
        .collect();
    ChartDataPacket {
        lines,
        week_labels: cat_data.weeks.clone(),
        is_loading: false,
        show_x_axis_labels: true,
        show_y_axis_labels: true,
        show_end_labels: true,
    }
}

pub(super) fn render_preview_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    let (ai_warning_title, ai_warning_body) = view
        .ai_warning_banner
        .as_ref()
        .map(|banner| (banner.title.clone(), banner.body.clone()))
        .unwrap_or_else(|| (String::new(), String::new()));
    emit_if_changed(
        &mut tree_state.preview.prev_ai_warning_title_text,
        ai_warning_title,
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_AI_WARNING_TITLE,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.preview.prev_ai_warning_body_text,
        ai_warning_body,
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_AI_WARNING_BODY,
            text,
        },
    );

    // Summary tab: only show the selected article summary; never fall back to shared preview text.
    let summary_markdown = view
        .right_pane
        .summary_markdown
        .as_deref()
        .unwrap_or(SUMMARY_EMPTY_STATE_MARKDOWN);
    if tree_state.preview.prev_preview_text.as_deref() != Some(summary_markdown) {
        let (truncated_markdown, was_truncated) = truncate_markdown_for_preview(summary_markdown);
        let mut rtf_text = convert_markdown_to_rtf(&truncated_markdown);
        if was_truncated {
            engine_warn!(
                "[preview] summary markdown truncated from {} chars to {} chars",
                summary_markdown.chars().count(),
                truncated_markdown.chars().count()
            );
            if rtf_text.ends_with('}') {
                rtf_text.pop();
            }
            rtf_text.push_str("\\par ");
            rtf_text.push_str(RTF_TRUNCATE_MARKER);
            rtf_text.push('}');
        }
        cmds.push(PlatformCommand::SetRichEditContent {
            window_id,
            control_id: VIEWER_PREVIEW,
            rtf_text,
        });
        tree_state.preview.prev_preview_text = Some(summary_markdown.to_string());
    }

    // Triage tab viewer.
    let triage_markdown = view
        .right_pane
        .triage_markdown
        .as_deref()
        .unwrap_or_default();
    if tree_state.preview.prev_triage_text.as_deref() != Some(triage_markdown) {
        let (truncated, _) = truncate_markdown_for_preview(triage_markdown);
        cmds.push(PlatformCommand::SetRichEditContent {
            window_id,
            control_id: VIEWER_TRIAGE,
            rtf_text: convert_markdown_to_rtf(&truncated),
        });
        tree_state.preview.prev_triage_text = Some(triage_markdown.to_string());
    }

    // Briefing tab viewer.
    let briefing_markdown = view
        .right_pane
        .briefing_markdown
        .as_deref()
        .unwrap_or_default();
    if tree_state.preview.prev_briefing_text.as_deref() != Some(briefing_markdown) {
        let (truncated, _) = truncate_markdown_for_preview(briefing_markdown);
        let display = strip_leading_h1(&truncated);
        cmds.push(PlatformCommand::SetRichEditContent {
            window_id,
            control_id: VIEWER_BRIEFING,
            rtf_text: convert_markdown_to_rtf(display),
        });
        tree_state.preview.prev_briefing_text = Some(briefing_markdown.to_string());
    }

    // Poll Stats tab viewer.
    let poll_stats_text = view
        .right_pane
        .poll_stats_markdown
        .as_deref()
        .unwrap_or("No poll data yet.");
    if tree_state.preview.prev_poll_stats_text.as_deref() != Some(poll_stats_text) {
        cmds.push(PlatformCommand::SetRichEditContent {
            window_id,
            control_id: VIEWER_POLL_STATS,
            rtf_text: convert_markdown_to_rtf(poll_stats_text),
        });
        tree_state.preview.prev_poll_stats_text = Some(poll_stats_text.to_string());
    }

    // Trends tab: category selector.
    let active_category = view.right_pane.trends.active_category;
    cmds.push(PlatformCommand::SetTabBarSelection {
        window_id,
        control_id: TAB_BAR_TRENDS,
        selected_index: active_category.to_index(),
    });

    // Trends tab: chart data (unconditional — mirrors radio button pattern; InvalidateRect is cheap).
    cmds.push(PlatformCommand::SetChartData {
        window_id,
        control_id: CHART_TRENDS,
        data: build_chart_data(&view.right_pane.trends),
    });

    if let Some(header_text) = view.preview_header_text.clone() {
        emit_if_changed(
            &mut tree_state.preview.prev_preview_header_override_text,
            header_text,
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_HEADER,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_source_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_SOURCE,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_status_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_STATUS,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_attention_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_ATTENTION,
                text,
            },
        );
    } else if let Some(context) = view.preview_context.as_ref() {
        emit_if_changed(
            &mut tree_state.preview.prev_preview_header_override_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_HEADER,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_source_text,
            format_preview_source_label(&context.source_label),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_SOURCE,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_status_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_STATUS,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_attention_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_ATTENTION,
                text,
            },
        );
    } else {
        emit_if_changed(
            &mut tree_state.preview.prev_preview_header_override_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_HEADER,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_source_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_SOURCE,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_status_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_STATUS,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_attention_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_ATTENTION,
                text,
            },
        );
    }
}
