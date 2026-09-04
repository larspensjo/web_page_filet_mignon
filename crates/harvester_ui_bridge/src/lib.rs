//! Tauri-free IPC bridge between the desktop host and Harvester's core reducer.

pub mod assets;
pub mod driver;
pub mod ipc;
pub mod snapshot;

pub use assets::{resolve, ResolvedAsset, CSP};
pub use driver::{
    partition_effects, run_driver, DriverTermination, SnapshotCoalescer, SnapshotSignal, UiCommand,
    SNAPSHOT_MIN_INTERVAL_MS,
};
pub use ipc::{decode_intent, IPC_SCHEMA_VERSION};
pub use snapshot::{
    fetch_body, project, BodyKey, BodyRef, BodyTable, ProjectedSnapshot, SnapshotEnvelope,
};
