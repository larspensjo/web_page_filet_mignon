use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Archive-level entity index. Maps article URL → entity data.
/// Persisted as `.entity_index.ron` in the output directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityIndex {
    pub schema_version: u32,
    /// Keyed by article URL (exact string, no normalization at index level).
    pub entries: BTreeMap<String, EntityIndexEntry>,
}

impl Default for EntityIndex {
    fn default() -> Self {
        Self {
            schema_version: 1,
            entries: BTreeMap::new(),
        }
    }
}

/// Entity data for a single article.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EntityIndexEntry {
    /// RFC3339 UTC timestamp when the article was fetched.
    /// `None` until `LoadedArticle.fetched_utc` is wired in Slice 4.
    pub fetched_utc: Option<String>,
    /// SHA256 content hash; used as a cache join key.
    /// `None` if not yet available.
    pub content_hash: Option<String>,
    /// Named companies extracted by the V4+ summary prompt.
    pub companies: Vec<String>,
    /// Named technology categories extracted by the V4+ summary prompt.
    pub technologies: Vec<String>,
    /// Named branded products extracted by the V4+ summary prompt.
    pub products: Vec<String>,
    /// Theme tags from the triage step.
    pub themes: Vec<String>,
}
