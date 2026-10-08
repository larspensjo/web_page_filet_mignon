//! Manual decisions are fixture tools; the desktop runs automatic pre-triage.
use super::*;

#[doc(hidden)]
pub trait ManualPreTriageDecisions {
    fn set_manual_decision(
        &mut self,
        key: &ArticleFilterKey,
        decision: ManualDecision,
    ) -> Result<(), &'static str>;
    fn clear_manual_decisions(&mut self);
    fn apply_manual_overrides(&mut self, overrides: &HashMap<ArticleFilterKey, ManualDecision>);
    fn manual_overrides(&self) -> HashMap<ArticleFilterKey, ManualDecision>;
}
impl ManualPreTriageDecisions for PreTriageSession {
    fn set_manual_decision(
        &mut self,
        key: &ArticleFilterKey,
        decision: ManualDecision,
    ) -> Result<(), &'static str> {
        if !self.is_interactive() {
            return Err("manual decisions are only allowed while reviewing");
        }
        let Some(entry) = self.entries.iter_mut().find(|entry| &entry.key == key) else {
            return Err("filter key not found");
        };
        entry.manual_decision = Some(decision);
        self.refresh_unresolved_review_count();
        self.refresh_loaded_lifecycle("no included articles after manual decisions");
        Ok(())
    }
    fn clear_manual_decisions(&mut self) {
        for entry in &mut self.entries {
            entry.manual_decision = None;
        }
        self.refresh_unresolved_review_count();
        self.refresh_loaded_lifecycle("no articles passed pre-triage filters");
    }
    fn apply_manual_overrides(&mut self, overrides: &HashMap<ArticleFilterKey, ManualDecision>) {
        if overrides.is_empty() {
            self.refresh_loaded_lifecycle("no articles passed pre-triage filters");
            return;
        }
        for entry in &mut self.entries {
            entry.manual_decision = overrides.get(&entry.key).copied();
        }
        self.refresh_unresolved_review_count();
        self.refresh_loaded_lifecycle("no articles passed pre-triage filters");
    }
    fn manual_overrides(&self) -> HashMap<ArticleFilterKey, ManualDecision> {
        self.entries
            .iter()
            .filter_map(|entry| {
                entry
                    .manual_decision
                    .map(|decision| (entry.key.clone(), decision))
            })
            .collect()
    }
}
