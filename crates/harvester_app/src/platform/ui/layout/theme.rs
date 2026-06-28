use commanductui::{
    Color, ControlStyle, FontDescription, FontWeight, PlatformCommand, StyleId, TextAlignment,
    WindowId,
};

use super::super::constants::*;
use super::super::groups::{bottom_buttons, prompt_lab_actions, prompt_lab_sections};

pub(super) fn define_dark_theme_styles(commands: &mut Vec<PlatformCommand>) {
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::MainWindowBackground,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x14,
                g: 0x14,
                b: 0x13,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::PanelBackground,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1E,
                g: 0x1E,
                b: 0x1C,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::StatusBarBackground,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x14,
                g: 0x14,
                b: 0x13,
            }),
            text_color: Some(Color {
                r: 0x87,
                g: 0x86,
                b: 0x7F,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::StatusLabelWarning,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x4A,
                g: 0x33,
                b: 0x1D,
            }),
            text_color: Some(Color {
                r: 0xFF,
                g: 0xF1,
                b: 0xD6,
            }),
            font: Some(FontDescription {
                name: Some("Segoe UI".to_string()),
                size: Some(10),
                weight: Some(FontWeight::Normal),
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::DefaultText,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x14,
                g: 0x14,
                b: 0x13,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::HeaderLabel,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1E,
                g: 0x1E,
                b: 0x1C,
            }),
            text_color: Some(Color {
                r: 0xC9,
                g: 0x64,
                b: 0x42,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::MetadataText,
        style: ControlStyle {
            background_color: None,
            text_color: Some(Color {
                r: 0x87,
                g: 0x86,
                b: 0x7F,
            }),
            font: Some(FontDescription {
                name: Some("Segoe UI".to_string()),
                size: Some(12),
                weight: Some(FontWeight::Normal),
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::DefaultInput,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x30,
                g: 0x30,
                b: 0x2E,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::DefaultButton,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x18,
                g: 0x18,
                b: 0x17,
            }),
            text_color: Some(Color {
                r: 0xB0,
                g: 0xAE,
                b: 0xA5,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::SecondaryButton,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x2A,
                g: 0x2A,
                b: 0x28,
            }),
            text_color: Some(Color {
                r: 0xB0,
                g: 0xAE,
                b: 0xA5,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::LinkButton,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1E,
                g: 0x1E,
                b: 0x1C,
            }),
            text_color: Some(Color {
                r: 0xC9,
                g: 0x64,
                b: 0x42,
            }),
            font: Some(FontDescription {
                name: Some("Segoe UI".to_string()),
                size: Some(12),
                weight: Some(FontWeight::Normal),
            }),
            text_alignment: Some(TextAlignment::Left),
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::RadioButton,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1E,
                g: 0x1E,
                b: 0x1C,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            ..Default::default()
        },
    });
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::CheckBox,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1E,
                g: 0x1E,
                b: 0x1C,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            ..Default::default()
        },
    });

    // TabBar background and text colors.
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TabBar,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x14,
                g: 0x14,
                b: 0x13,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            ..Default::default()
        },
    });
    // TabBarAccent: the bottom accent line color (background_color carries the value).
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TabBarAccent,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0xC9,
                g: 0x64,
                b: 0x42,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TreeView,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1E,
                g: 0x1E,
                b: 0x1C,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ViewerMonospace,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x30,
                g: 0x30,
                b: 0x2E,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            font: Some(FontDescription {
                name: Some("Cascadia Code".to_string()),
                size: Some(10),
                weight: Some(FontWeight::Normal),
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ViewerReadable,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x30,
                g: 0x30,
                b: 0x2E,
            }),
            text_color: Some(Color {
                r: 0xB0,
                g: 0xAE,
                b: 0xA5,
            }),
            font: Some(FontDescription {
                name: Some("Segoe UI".to_string()),
                size: Some(12),
                weight: Some(FontWeight::Normal),
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ProgressBar,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x30,
                g: 0x30,
                b: 0x2E,
            }),
            text_color: Some(Color {
                r: 0xC9,
                g: 0x64,
                b: 0x42,
            }),
            ..Default::default()
        },
    });

    // Splitter control style: neutral gray matching the theme
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::Splitter,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x2A,
                g: 0x2A,
                b: 0x28,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ComboBox,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x30,
                g: 0x30,
                b: 0x2E,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            ..Default::default()
        },
    });

    // Muted gray for tree items without summaries
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TreeItemDisabled,
        style: ControlStyle {
            text_color: Some(Color {
                r: 0x87,
                g: 0x86,
                b: 0x7F,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ListBoxRow,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1E,
                g: 0x1E,
                b: 0x1C,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            ..Default::default()
        },
    });
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ListBoxSelectedRow,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x2C,
                g: 0x23,
                b: 0x21,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            ..Default::default()
        },
    });
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ListBoxSelectionAccent,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0xC9,
                g: 0x64,
                b: 0x42,
            }),
            ..Default::default()
        },
    });
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ListBoxHoverRow,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x2A,
                g: 0x2A,
                b: 0x28,
            }),
            ..Default::default()
        },
    });
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ListBoxDisabledRow,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1E,
                g: 0x1E,
                b: 0x1C,
            }),
            text_color: Some(Color {
                r: 0x5E,
                g: 0x5D,
                b: 0x59,
            }),
            ..Default::default()
        },
    });

    for (style_id, background, text) in [
        (
            StyleId::BadgePriorityCritical,
            Color {
                r: 0x8F,
                g: 0x2D,
                b: 0x2E,
            },
            Color {
                r: 0xFF,
                g: 0xF7,
                b: 0xF4,
            },
        ),
        (
            StyleId::BadgePriorityHigh,
            Color {
                r: 0xA7,
                g: 0x72,
                b: 0x2A,
            },
            Color {
                r: 0xFF,
                g: 0xF8,
                b: 0xEA,
            },
        ),
        (
            StyleId::BadgePriorityMedium,
            Color {
                r: 0x66,
                g: 0x4B,
                b: 0x8D,
            },
            Color {
                r: 0xF1,
                g: 0xE9,
                b: 0xFF,
            },
        ),
        (
            StyleId::BadgePriorityLow,
            Color {
                r: 0x3D,
                g: 0x3D,
                b: 0x3A,
            },
            Color {
                r: 0xB0,
                g: 0xAE,
                b: 0xA5,
            },
        ),
        (
            StyleId::BadgeCategory,
            Color {
                r: 0x3D,
                g: 0x45,
                b: 0x43,
            },
            Color {
                r: 0xD6,
                g: 0xDC,
                b: 0xD7,
            },
        ),
        (
            StyleId::BadgeStatusDone,
            Color {
                r: 0x2C,
                g: 0x6C,
                b: 0x4A,
            },
            Color {
                r: 0xEC,
                g: 0xF8,
                b: 0xEF,
            },
        ),
        (
            StyleId::BadgeStatusError,
            Color {
                r: 0x8C,
                g: 0x36,
                b: 0x36,
            },
            Color {
                r: 0xFF,
                g: 0xF0,
                b: 0xF0,
            },
        ),
        (
            StyleId::BadgeStatusActive,
            Color {
                r: 0x5A,
                g: 0x4A,
                b: 0x8F,
            },
            Color {
                r: 0xF4,
                g: 0xEF,
                b: 0xFF,
            },
        ),
        (
            StyleId::BadgeStatusMuted,
            Color {
                r: 0x54,
                g: 0x58,
                b: 0x5E,
            },
            Color {
                r: 0xE0,
                g: 0xE5,
                b: 0xEC,
            },
        ),
        (
            StyleId::BadgeIndirect,
            Color {
                r: 0x54,
                g: 0x58,
                b: 0x5E,
            },
            Color {
                r: 0xE0,
                g: 0xE5,
                b: 0xEC,
            },
        ),
    ] {
        commands.push(PlatformCommand::DefineStyle {
            style_id,
            style: ControlStyle {
                background_color: Some(background),
                text_color: Some(text),
                ..Default::default()
            },
        });
    }

    // Selected row: subtle lighter background
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TreeViewSelectedRow,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x3D,
                g: 0x3D,
                b: 0x3A,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            ..Default::default()
        },
    });

    // Accent bar: blue accent matching TabBar accent
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TreeViewSelectionAccent,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0xC9,
                g: 0x64,
                b: 0x42,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::StatusMeter,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x14,
                g: 0x14,
                b: 0x13,
            }),
            text_color: Some(Color {
                r: 0x87,
                g: 0x86,
                b: 0x7F,
            }),
            font: Some(FontDescription {
                name: Some("Segoe UI".to_string()),
                size: Some(10),
                weight: Some(FontWeight::Normal),
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::SectionTitle,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1E,
                g: 0x1E,
                b: 0x1C,
            }),
            text_color: Some(Color {
                r: 0xB0,
                g: 0xAE,
                b: 0xA5,
            }),
            font: Some(FontDescription {
                name: Some("Segoe UI".to_string()),
                size: Some(12),
                weight: Some(FontWeight::Bold),
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::PrimaryButton,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0xC9,
                g: 0x64,
                b: 0x42,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::DestructiveButton,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0xB5,
                g: 0x33,
                b: 0x33,
            }),
            text_color: Some(Color {
                r: 0xFA,
                g: 0xF9,
                b: 0xF5,
            }),
            ..Default::default()
        },
    });
}

pub(super) fn apply_dark_theme(window_id: WindowId, commands: &mut Vec<PlatformCommand>) {
    for control_id in [
        PANEL_TOOLBAR,
        PANEL_PROGRESS,
        PANEL_BUTTONS,
        PANEL_INPUT,
        PANEL_JOBS,
        PANEL_PREVIEW,
        PANEL_AI_WARNING,
        PANEL_TAB_TRIAGE,
        PANEL_TAB_SUMMARY,
        PANEL_TAB_BRIEFING,
        PANEL_TAB_TRENDS,
        PANEL_TAB_POLL_STATS,
        PANEL_LEFT,
        PANEL_LEFT_JOBS,
        PANEL_LEFT_PROMPT_LAB,
        PANEL_LEFT_BLACKLIST,
        PANEL_PROMPT_LAB,
        PANEL_PROMPT_LAB_MODE_ROW,
        PANEL_PROMPT_LAB_MODEL_ROW,
        PANEL_PROMPT_LAB_STAGE_ROW,
        PANEL_PROMPT_LAB_SOURCE_ROW,
        PANEL_PROMPT_LAB_INPUT_ROW,
        PANEL_PROMPT_LAB_ACTION_ROW,
        PANEL_PROMPT_LAB_COMPARE_HEADER_ROW,
        PANEL_PROMPT_LAB_COMPARE_ROW,
        PANEL_PROMPT_LAB_CONTEXT_HEADER_ROW,
        PANEL_PROMPT_LAB_CONTEXT_ROW,
        PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW,
        PANEL_PROMPT_LAB_TEMPLATE_HEADER_ROW,
        PANEL_PROMPT_LAB_RUN_DETAILS_HEADER_ROW,
        PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW,
        PANEL_PROMPT_LAB_TEMPLATE_SYSTEM_ROW,
        PANEL_PROMPT_LAB_TEMPLATE_USER_ROW,
    ] {
        commands.push(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id,
            style_id: StyleId::PanelBackground,
        });
    }

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: PANEL_BOTTOM,
        style_id: StyleId::StatusBarBackground,
    });

    commands.push(PlatformCommand::SetToggleSwitchStyle {
        window_id,
        control_id: TS_JOBS_SCOPE,
        background: Color {
            r: 0x14,
            g: 0x14,
            b: 0x13,
        },
        pill_off: Color {
            r: 0x3D,
            g: 0x3D,
            b: 0x3A,
        },
        pill_on: Color {
            r: 0x3D,
            g: 0x3D,
            b: 0x3A,
        },
        knob: Color {
            r: 0xC9,
            g: 0x64,
            b: 0x42,
        },
        text: Color {
            r: 0x87,
            g: 0x86,
            b: 0x7F,
        },
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_PREVIEW_HEADER,
        style_id: StyleId::SectionTitle,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: PANEL_AI_WARNING,
        style_id: StyleId::StatusLabelWarning,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_AI_WARNING_TITLE,
        style_id: StyleId::StatusLabelWarning,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_AI_WARNING_BODY,
        style_id: StyleId::StatusLabelWarning,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: PANEL_PREVIEW_CONTEXT,
        style_id: StyleId::PanelBackground,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_JOBS_HEADER_TITLE,
        style_id: StyleId::SectionTitle,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_JOBS_HEADER_META,
        style_id: StyleId::MetadataText,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_PREVIEW_SOURCE_CAPTION,
        style_id: StyleId::MetadataText,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: BUTTON_PREVIEW_SOURCE_LINK,
        style_id: StyleId::LinkButton,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_PREVIEW_STATUS,
        style_id: StyleId::MetadataText,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_PREVIEW_ATTENTION,
        style_id: StyleId::MetadataText,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_TRENDS_DESCRIPTION,
        style_id: StyleId::MetadataText,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_SIGNAL_CANDIDATE_STATE,
        style_id: StyleId::MetadataText,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_SIGNAL_CANDIDATE_CLUSTER_CAPTION,
        style_id: StyleId::MetadataText,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_SIGNAL_CANDIDATE_CLUSTER_URLS,
        style_id: StyleId::MetadataText,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_TOKEN_PROGRESS,
        style_id: StyleId::StatusMeter,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_TOKEN_COUNTS,
        style_id: StyleId::MetadataText,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_INPUT_HINT,
        style_id: StyleId::HeaderLabel,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_STATUS,
        style_id: StyleId::StatusBarBackground,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_OPERATION_PROGRESS,
        style_id: StyleId::StatusBarBackground,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_LLM_QUOTA,
        style_id: StyleId::StatusBarBackground,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_PROMPT_LAB_STATUS,
        style_id: StyleId::HeaderLabel,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_PROMPT_LAB_METADATA,
        style_id: StyleId::DefaultText,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: INPUT_URLS,
        style_id: StyleId::DefaultInput,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: INPUT_PROMPT_LAB_URL,
        style_id: StyleId::DefaultInput,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: INPUT_JOBS_SEARCH,
        style_id: StyleId::DefaultInput,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: COMBO_PROMPT_LAB_MODEL_SELECTOR,
        style_id: StyleId::ComboBox,
    });
    for control_id in [
        VIEWER_PREVIEW,
        VIEWER_TRIAGE,
        VIEWER_BRIEFING,
        VIEWER_POLL_STATS,
        VIEWER_BLACKLIST,
    ] {
        commands.push(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id,
            style_id: StyleId::ViewerReadable,
        });
    }

    bottom_buttons::apply_theme(window_id, commands);
    prompt_lab_actions::apply_theme(window_id, commands);

    for control_id in [
        BTN_PROMPT_LAB_MODE_BASIC,
        BTN_PROMPT_LAB_MODE_ADVANCED,
        BTN_STAGE_TRIAGE,
        BTN_STAGE_SUMMARY,
        BTN_STAGE_BRIEFING,
    ] {
        commands.push(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id,
            style_id: StyleId::RadioButton,
        });
    }
    prompt_lab_sections::apply_theme(window_id, commands);
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: CHK_SIGNAL_CANDIDATE_EXCLUDE,
        style_id: StyleId::CheckBox,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: INPUT_PROMPT_LAB_CONTEXT,
        style_id: StyleId::DefaultInput,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: INPUT_PROMPT_LAB_TEMPLATE_SYSTEM,
        style_id: StyleId::DefaultInput,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: INPUT_PROMPT_LAB_TEMPLATE_USER,
        style_id: StyleId::DefaultInput,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: TREE_JOBS,
        style_id: StyleId::ListBoxRow,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: TREE_JOBS,
        style_id: StyleId::ListBoxSelectedRow,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: TREE_JOBS,
        style_id: StyleId::ListBoxSelectionAccent,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: TREE_JOBS,
        style_id: StyleId::ListBoxHoverRow,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: TREE_JOBS,
        style_id: StyleId::ListBoxDisabledRow,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: PROGRESS_TOKENS,
        style_id: StyleId::StatusMeter,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: PROGRESS_OPERATION,
        style_id: StyleId::ProgressBar,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: PROGRESS_LLM_QUOTA,
        style_id: StyleId::StatusMeter,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: SPLITTER_MAIN,
        style_id: StyleId::Splitter,
    });
}
