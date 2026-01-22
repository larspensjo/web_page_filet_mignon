use std::fs;
use std::path::{Path, PathBuf};

use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_core::CompletedJobSnapshot;
use harvester_engine::{ensure_output_dir, AtomicFileWriter};
use serde::{Deserialize, Serialize};

const STATE_FILENAME: &str = ".harvester_state.ron";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedJob {
    url: String,
    tokens: Option<u32>,
    bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedState {
    completed: Vec<PersistedJob>,
}

pub(crate) fn load_completed_jobs(output_dir: &Path) -> Vec<CompletedJobSnapshot> {
    let path = output_dir.join(STATE_FILENAME);
    let content = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Vec::new();
        }
        Err(err) => {
            engine_warn!(
                "Failed to read persisted state from {:?}: {}",
                path,
                err
            );
            return Vec::new();
        }
    };

    let state: PersistedState = match ron::from_str(&content) {
        Ok(state) => state,
        Err(err) => {
            engine_warn!("Failed to parse persisted state from {:?}: {}", path, err);
            return Vec::new();
        }
    };

    let completed = state
        .completed
        .into_iter()
        .map(|job| CompletedJobSnapshot {
            url: job.url,
            tokens: job.tokens,
            bytes: job.bytes,
        })
        .collect();

    engine_info!("Loaded persisted completed jobs from {:?}", path);
    completed
}

pub(crate) fn save_completed_jobs(output_dir: &Path, completed: &[CompletedJobSnapshot]) {
    if let Err(err) = ensure_output_dir(output_dir) {
        engine_error!("Failed to ensure output dir {:?}: {}", output_dir, err);
        return;
    }

    let state = PersistedState {
        completed: completed
            .iter()
            .map(|job| PersistedJob {
                url: job.url.clone(),
                tokens: job.tokens,
                bytes: job.bytes,
            })
            .collect(),
    };

    let pretty = ron::ser::PrettyConfig::new();
    let content = match ron::ser::to_string_pretty(&state, pretty) {
        Ok(text) => text,
        Err(err) => {
            engine_error!("Failed to serialize persisted state: {}", err);
            return;
        }
    };

    let writer = AtomicFileWriter::new(PathBuf::from(output_dir));
    if let Err(err) = writer.write(STATE_FILENAME, &content) {
        engine_error!(
            "Failed to write persisted state to {:?}: {}",
            output_dir,
            err
        );
    }
}
