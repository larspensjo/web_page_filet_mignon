//! Harvester engine: IO pipeline placeholder for Phase 0.

#[derive(Debug, Clone, Copy, Default)]
pub struct EngineHandle;

impl EngineHandle {
    pub fn new() -> Self {
        EngineHandle
    }
}

/// Lightweight version identifier for logging/tests.
pub const VERSION: &str = "0.0.0-phase0";
