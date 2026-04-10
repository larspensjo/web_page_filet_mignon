use commanductui::types::ControlId;

pub const INPUT_URLS: ControlId = ControlId::new(1001);
pub const BUTTON_STOP: ControlId = ControlId::new(1003);
pub const BUTTON_BRIEFING: ControlId = ControlId::new(1005);
pub const BUTTON_TRIAGE: ControlId = ControlId::new(1007);
pub const BUTTON_POLL_SOURCES: ControlId = ControlId::new(1008);
pub const BUTTON_OPEN_BROWSER: ControlId = ControlId::new(1009);
pub const BUTTON_ARCHIVE: ControlId = ControlId::new(1010);
pub const TREE_JOBS: ControlId = ControlId::new(1501);

pub const BTN_STAGE_TRIAGE: ControlId = ControlId::new(1011);
pub const BTN_STAGE_SUMMARY: ControlId = ControlId::new(1012);
pub const BTN_STAGE_BRIEFING: ControlId = ControlId::new(1013);
pub const BTN_SOURCE_FROM_TRIAGE: ControlId = ControlId::new(1014);
pub const BTN_SOURCE_TYPE_URL: ControlId = ControlId::new(1015);
pub const INPUT_PROMPT_LAB_URL: ControlId = ControlId::new(1016);
pub const BTN_PROMPT_LAB_RESOLVE: ControlId = ControlId::new(1017);
pub const BTN_PROMPT_LAB_RUN: ControlId = ControlId::new(1018);
pub const PANEL_BOTTOM: ControlId = ControlId::new(2001);
pub const PANEL_INPUT: ControlId = ControlId::new(2002);
pub const PANEL_PROGRESS: ControlId = ControlId::new(2003);
pub const PANEL_BUTTONS: ControlId = ControlId::new(2004);
pub const PANEL_PREVIEW: ControlId = ControlId::new(2005);
pub const PANEL_JOBS: ControlId = ControlId::new(2006);
pub const PANEL_PREVIEW_CONTEXT: ControlId = ControlId::new(2007);
pub const PANEL_TOOLBAR: ControlId = ControlId::new(2015);

pub const PANEL_PROMPT_LAB: ControlId = ControlId::new(2100);
pub const PANEL_PROMPT_LAB_STAGE_ROW: ControlId = ControlId::new(2101);
pub const PANEL_PROMPT_LAB_SOURCE_ROW: ControlId = ControlId::new(2102);
pub const PANEL_PROMPT_LAB_INPUT_ROW: ControlId = ControlId::new(2103);
pub const PANEL_PROMPT_LAB_ACTION_ROW: ControlId = ControlId::new(2104);
pub const PANEL_PROMPT_LAB_CONTEXT_ROW: ControlId = ControlId::new(2105);
pub const PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW: ControlId = ControlId::new(2106);
pub const INPUT_PROMPT_LAB_CONTEXT: ControlId = ControlId::new(2107);
pub const BTN_PROMPT_LAB_CONTEXT_APPLY: ControlId = ControlId::new(2108);
pub const BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN: ControlId = ControlId::new(2109);
pub const BTN_PROMPT_LAB_CONTEXT_REVERT: ControlId = ControlId::new(2110);
pub const BTN_PROMPT_LAB_CONTEXT_SAVE: ControlId = ControlId::new(2111);
pub const BTN_PROMPT_LAB_CONTEXT_RELOAD: ControlId = ControlId::new(2112);
pub const PANEL_PROMPT_LAB_TEMPLATE_SYSTEM_ROW: ControlId = ControlId::new(2113);
pub const PANEL_PROMPT_LAB_TEMPLATE_USER_ROW: ControlId = ControlId::new(2114);
pub const PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW: ControlId = ControlId::new(2115);
pub const INPUT_PROMPT_LAB_TEMPLATE_SYSTEM: ControlId = ControlId::new(2116);
pub const INPUT_PROMPT_LAB_TEMPLATE_USER: ControlId = ControlId::new(2117);
pub const CHK_PROMPT_LAB_TEMPLATE_OPEN: ControlId = ControlId::new(2118);
pub const BTN_PROMPT_LAB_TEMPLATE_APPLY: ControlId = ControlId::new(2119);
pub const BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN: ControlId = ControlId::new(2120);
pub const BTN_PROMPT_LAB_TEMPLATE_REVERT: ControlId = ControlId::new(2121);
pub const BTN_PROMPT_LAB_TEMPLATE_SAVE: ControlId = ControlId::new(2122);
pub const PANEL_PROMPT_LAB_COMPARE_ROW: ControlId = ControlId::new(2123);
pub const PANEL_PROMPT_LAB_MODE_ROW: ControlId = ControlId::new(2124);
pub const PANEL_PROMPT_LAB_COMPARE_HEADER_ROW: ControlId = ControlId::new(2125);
pub const PANEL_PROMPT_LAB_CONTEXT_HEADER_ROW: ControlId = ControlId::new(2126);
pub const PANEL_PROMPT_LAB_TEMPLATE_HEADER_ROW: ControlId = ControlId::new(2127);
pub const PANEL_PROMPT_LAB_RUN_DETAILS_HEADER_ROW: ControlId = ControlId::new(2128);
pub const PANEL_PROMPT_LAB_MODEL_ROW: ControlId = ControlId::new(2129);
pub const LABEL_STATUS: ControlId = ControlId::new(3001);
pub const LABEL_INPUT_HINT: ControlId = ControlId::new(3002);
pub const LABEL_TOKEN_PROGRESS: ControlId = ControlId::new(3003);
pub const LABEL_PREVIEW_HEADER: ControlId = ControlId::new(3004);
pub const LABEL_JOBS_HEADER_TITLE: ControlId = ControlId::new(3005);
#[allow(dead_code)]
pub const LABEL_JOBS_HEADER: ControlId = LABEL_JOBS_HEADER_TITLE;
pub const LABEL_OPERATION_PROGRESS: ControlId = ControlId::new(3006);
pub const LABEL_JOBS_HEADER_META: ControlId = ControlId::new(3007);
pub const LABEL_PREVIEW_SOURCE: ControlId = ControlId::new(3008);
pub const LABEL_PREVIEW_STATUS: ControlId = ControlId::new(3009);

pub const LABEL_TRENDS_DESCRIPTION: ControlId = ControlId::new(3014);
pub const LABEL_PROMPT_LAB_STATUS: ControlId = ControlId::new(3010);
pub const LABEL_PROMPT_LAB_METADATA: ControlId = ControlId::new(3011);
pub const LABEL_PROMPT_LAB_CONTEXT_STATUS: ControlId = ControlId::new(3012);
pub const LABEL_PROMPT_LAB_TEMPLATE_STATUS: ControlId = ControlId::new(3013);
pub const TS_JOBS_SCOPE: ControlId = ControlId::new(3020);
pub const LABEL_PREVIEW_ATTENTION: ControlId = ControlId::new(3015);
pub const BTN_COMPARE_ADD_CURRENT: ControlId = ControlId::new(3100);
pub const BTN_COMPARE_ADD_BASELINE: ControlId = ControlId::new(3101);
pub const BTN_COMPARE_RESET_DRAFT: ControlId = ControlId::new(3102);
pub const BTN_COMPARE_START: ControlId = ControlId::new(3103);
pub const BTN_COMPARE_CANCEL: ControlId = ControlId::new(3104);
pub const BTN_COMPARE_AUTO_SELECT: ControlId = ControlId::new(3105);
pub const BTN_COMPARE_WINNER_CLEAR: ControlId = ControlId::new(3106);
pub const BTN_PROMPT_LAB_MODE_BASIC: ControlId = ControlId::new(3107);
pub const BTN_PROMPT_LAB_MODE_ADVANCED: ControlId = ControlId::new(3108);
pub const CHK_PROMPT_LAB_SECTION_COMPARE: ControlId = ControlId::new(3109);
pub const CHK_PROMPT_LAB_SECTION_CONTEXT: ControlId = ControlId::new(3110);
pub const CHK_PROMPT_LAB_SECTION_TEMPLATE: ControlId = ControlId::new(3111);
pub const CHK_PROMPT_LAB_SECTION_RUN_DETAILS: ControlId = ControlId::new(3112);
pub const COMBO_PROMPT_LAB_MODEL_SELECTOR: ControlId = ControlId::new(3113);
pub const PROGRESS_TOKENS: ControlId = ControlId::new(4001);
pub const PROGRESS_OPERATION: ControlId = ControlId::new(4002);
pub const VIEWER_PREVIEW: ControlId = ControlId::new(5001);
pub const SPLITTER_MAIN: ControlId = ControlId::new(6001);

// Tab content panels (2200 range)
pub const PANEL_TAB_TRIAGE: ControlId = ControlId::new(2210);
pub const PANEL_TAB_SUMMARY: ControlId = ControlId::new(2211);
pub const PANEL_TAB_BRIEFING: ControlId = ControlId::new(2212);
pub const PANEL_TAB_TRENDS: ControlId = ControlId::new(2213);
pub const PANEL_TAB_POLL_STATS: ControlId = ControlId::new(2214);
// RichEdit viewers inside each tab (except PromptLab which reuses existing controls)
pub const VIEWER_TRIAGE: ControlId = ControlId::new(5002);
pub const VIEWER_BRIEFING: ControlId = ControlId::new(5003);
pub const VIEWER_POLL_STATS: ControlId = ControlId::new(5004);
// GDI chart control for the Trends tab
pub const CHART_TRENDS: ControlId = ControlId::new(2230);
// Left panel: two tab content areas (2300 range)
pub const PANEL_LEFT: ControlId = ControlId::new(2300);
pub const PANEL_LEFT_JOBS: ControlId = ControlId::new(2304);
pub const PANEL_LEFT_PROMPT_LAB: ControlId = ControlId::new(2305);

// New custom TabBar controls replacing the radio-button tab bars (6000 range).
pub const TAB_BAR_RIGHT: ControlId = ControlId::new(6100);
pub const TAB_BAR_LEFT: ControlId = ControlId::new(6101);
pub const TAB_BAR_TRENDS: ControlId = ControlId::new(6102);
