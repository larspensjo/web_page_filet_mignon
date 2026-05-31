use harvester_core::{AppViewModel, LayoutViewModel, Msg};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RenderMode {
    Full,
    LayoutOnly,
}

pub(super) enum PendingRender {
    Full(Box<AppViewModel>),
    LayoutOnly(LayoutViewModel),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct GeometryBatchStats {
    pub(super) splitter_moves: usize,
    pub(super) window_resizes: usize,
    pub(super) last_splitter_width: Option<i32>,
    pub(super) last_window_width: Option<i32>,
}

impl GeometryBatchStats {
    pub(super) fn observe(&mut self, msg: &Msg) {
        match msg {
            Msg::SplitterMoved {
                desired_left_width_px,
            } => {
                self.splitter_moves += 1;
                self.last_splitter_width = Some(*desired_left_width_px);
            }
            Msg::WindowResized { window_width } => {
                self.window_resizes += 1;
                self.last_window_width = Some(*window_width);
            }
            _ => {}
        }
    }

    pub(super) fn is_empty(self) -> bool {
        self.splitter_moves == 0 && self.window_resizes == 0
    }
}

pub(super) fn is_geometry_only_message(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::SplitterMoved { .. } | Msg::WindowResized { .. } | Msg::WindowResizeCompleted { .. }
    )
}

pub(super) fn select_render_mode(
    any_dirty: bool,
    geometry_only_batch: bool,
    refresh_evaluation_dispatched: bool,
    clear_input_needed: bool,
    effect_count: usize,
) -> Option<RenderMode> {
    if !any_dirty {
        return None;
    }
    if geometry_only_batch
        && !refresh_evaluation_dispatched
        && !clear_input_needed
        && effect_count == 0
    {
        return Some(RenderMode::LayoutOnly);
    }
    Some(RenderMode::Full)
}
