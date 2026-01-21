use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;

use harvester_core::{Effect, JobResultKind, Msg, Stage, StopPolicy};

pub struct EffectRunner {
    effect_tx: mpsc::Sender<Effect>,
}

impl EffectRunner {
    pub fn new(msg_tx: mpsc::Sender<Msg>) -> Self {
        let (effect_tx, effect_rx) = mpsc::channel();
        let engine = EngineStub::new(msg_tx);
        thread::spawn(move || engine.run(effect_rx));
        Self { effect_tx }
    }

    pub fn enqueue(&self, effects: Vec<Effect>) {
        for effect in effects {
            let _ = self.effect_tx.send(effect);
        }
    }
}

struct EngineStub {
    msg_tx: mpsc::Sender<Msg>,
    stop_requested: Arc<AtomicBool>,
}

impl EngineStub {
    fn new(msg_tx: mpsc::Sender<Msg>) -> Self {
        Self {
            msg_tx,
            stop_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    fn run(self, effect_rx: mpsc::Receiver<Effect>) {
        while let Ok(effect) = effect_rx.recv() {
            match effect {
                Effect::StartSession => {
                    self.stop_requested.store(false, Ordering::Relaxed);
                }
                Effect::StopFinish { policy } => {
                    if matches!(policy, StopPolicy::Immediate | StopPolicy::Finish) {
                        self.stop_requested.store(true, Ordering::Relaxed);
                    }
                }
                Effect::EnqueueUrl { job_id, url } => {
                    let msg_tx = self.msg_tx.clone();
                    let stop_requested = self.stop_requested.clone();
                    thread::spawn(move || {
                        simulate_job(job_id, url, msg_tx, stop_requested);
                    });
                }
            }
        }
    }
}

fn simulate_job(
    job_id: harvester_core::JobId,
    url: String,
    msg_tx: mpsc::Sender<Msg>,
    _stop_requested: Arc<AtomicBool>,
) {
    let bytes = (url.len() as u64).saturating_mul(128);
    let tokens = (url.len() as u32).saturating_mul(2);

    let stages = [
        Stage::Downloading,
        Stage::Sanitizing,
        Stage::Converting,
        Stage::Tokenizing,
        Stage::Writing,
    ];

    for stage in stages {
        thread::sleep(Duration::from_millis(120));
        let _ = msg_tx.send(Msg::JobProgress {
            job_id,
            stage,
            tokens: if stage == Stage::Tokenizing {
                Some(tokens)
            } else {
                None
            },
            bytes: if stage == Stage::Downloading {
                Some(bytes)
            } else {
                None
            },
        });
    }

    let _ = msg_tx.send(Msg::JobDone {
        job_id,
        result: JobResultKind::Success,
    });
}
