//! Process-lifetime release history and the current run's admission boundary.
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PipelineRunScope {
    Full,
    Resume,
}

pub(crate) type Identity = (String, String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineWave {
    pub number: u64,
    pub run_id: u64,
    pub stage: crate::PipelineStage,
    pub members: Vec<Identity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PipelineWaves {
    pub(crate) waves: Vec<PipelineWave>,
    pub(crate) summary_released: HashSet<(Identity, crate::TriageCacheKey)>,
    pub(crate) scoring_released: HashSet<(Identity, String)>,
    pub(crate) pending: [VecDeque<Identity>; 3],
    pub(crate) triage_wave: HashMap<Identity, usize>,
    pub(crate) changed_triage_waves: BTreeSet<usize>,
}

impl PipelineWaves {
    pub fn waves(&self) -> &[PipelineWave] {
        &self.waves
    }

    pub(crate) fn push(
        &mut self,
        run_id: u64,
        stage: crate::PipelineStage,
        members: Vec<Identity>,
    ) {
        if members.is_empty() {
            return;
        }
        let number = self.waves.last().map_or(1, |w| w.number + 1);
        self.pending[stage.index() - 3].extend(members.iter().cloned());
        if stage == crate::PipelineStage::Triaging {
            let index = self.waves.len();
            for member in &members {
                self.triage_wave.insert(member.clone(), index);
            }
            self.changed_triage_waves.insert(index);
        }
        engine_logging::engine_info!(
            "[pipeline-wave] run_id={} stage={:?} wave={} released={}",
            run_id,
            stage,
            number,
            members.len()
        );
        self.waves.push(PipelineWave {
            number,
            run_id,
            stage,
            members,
        });
    }

    pub(crate) fn retain(&mut self, members: &HashSet<Identity>) {
        for wave in &mut self.waves {
            wave.members.retain(|m| members.contains(m));
        }
        self.summary_released.retain(|(m, _)| members.contains(m));
        self.scoring_released.retain(|(m, _)| members.contains(m));
        for pending in &mut self.pending {
            pending.retain(|m| members.contains(m));
        }
        self.triage_wave.retain(|m, _| members.contains(m));
        self.changed_triage_waves
            .extend(self.triage_wave.values().copied());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PipelineAdmission {
    pub scope: PipelineRunScope,
    pub armed: bool,
    pub configured: bool,
    pub intake_open: bool,
    pub fresh_load: bool,
    pub initial_admitted: bool,
    pub admitted: [HashSet<Identity>; 3],
    pub previous_window: HashSet<Identity>,
    pub reprocess_notice: Option<(usize, u64)>,
}

impl PipelineAdmission {
    pub(crate) fn new(
        scope: PipelineRunScope,
        armed: bool,
        previous_window: HashSet<Identity>,
    ) -> Self {
        Self {
            scope,
            armed,
            configured: false,
            intake_open: true,
            fresh_load: true,
            initial_admitted: false,
            admitted: Default::default(),
            previous_window,
            reprocess_notice: None,
        }
    }
}
