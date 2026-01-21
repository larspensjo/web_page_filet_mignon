use crate::view_model::AppViewModel;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppState {
    dirty: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn view(&self) -> AppViewModel {
        AppViewModel { dirty: self.dirty }
    }

    #[allow(dead_code)]
    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }
}
