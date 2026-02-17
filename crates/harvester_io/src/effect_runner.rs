// Placeholder for EffectRunner - will be fully implemented after initial structure is complete

use harvester_core::Effect;

/// Trait for platform-specific effect handling (e.g., opening URLs in browser)
pub trait PlatformEffectHandler: Send + Sync {
    fn open_url(&self, url: &str);
}

/// No-op handler for batch/headless mode
pub struct NoOpPlatformHandler;

impl PlatformEffectHandler for NoOpPlatformHandler {
    fn open_url(&self, _url: &str) {
        // No-op in batch mode
    }
}

/// Effect runner that orchestrates IO effects
pub struct EffectRunner {
    // Will be populated as we implement
}

impl EffectRunner {
    pub fn enqueue(&self, _effects: Vec<Effect>) {
        // Placeholder
        todo!("EffectRunner implementation pending")
    }
}
