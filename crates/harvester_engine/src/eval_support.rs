//! Narrow support seams for offline experiment crates.

use crate::ContentPrepConfig;

/// Returns the production default content-preparation configuration.
///
/// This is feature-gated so offline evaluators reuse the same clean-text
/// derivation without broadening Harvester Engine's default public surface.
pub fn default_content_prep_config() -> ContentPrepConfig {
    crate::briefing::build_content_prep_config()
}
