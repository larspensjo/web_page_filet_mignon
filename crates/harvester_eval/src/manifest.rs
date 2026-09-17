//! Frozen dataset manifest and integrity checks.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

use crate::identity::{IdentityAnomaly, PathKind};
use crate::prompt_identity::sha256;

pub const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub manifest_version: u32,
    pub tool_version: String,
    pub created_utc: String,
    pub source_output_dir: String,
    pub selection: Selection,
    pub frozen_inputs: FrozenInputs,
    pub counts: ManifestCounts,
    pub articles: Vec<ArticleEntry>,
    pub manifest_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Selection {
    pub prompt_id: String,
    pub prompt_version: u32,
    pub limit: usize,
    pub ordering: String,
    pub dev_share: f64,
    pub split_seed: u64,
    pub context_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrozenInputs {
    pub rubric_path: String,
    pub rubric_sha256: String,
    pub prompt_identity_path: String,
    pub system_sha256: String,
    pub template_sha256: String,
    pub context_version: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestCounts {
    pub records_scanned: u64,
    pub load_failed: u64,
    pub prompt_id_matched: u64,
    pub version_matched: u64,
    pub identity_matched: u64,
    pub validation_failed: u64,
    pub recovery_failed: u64,
    pub identity_anomalies: u64,
    pub distinct_articles: u64,
    pub exact_duplicates: u64,
    pub selected: u64,
    pub mapped: u64,
    pub unmapped: u64,
    pub ambiguous: u64,
    pub truncated: u64,
    pub duplicate_groups: u64,
    pub cross_path_duplicates: u64,
    pub sync_records: u64,
    pub batch_records: u64,
    pub instruction_like: u64,
    pub index_files_skipped: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArticleEntry {
    pub article_id: String,
    pub evidence_hash: String,
    pub source_content_hash: String,
    pub source_content_hashes: Vec<String>,
    pub path_kind: PathKind,
    pub text_path: String,
    pub text_bytes: usize,
    pub truncated: bool,
    pub nonce: String,
    pub nonce_verified: bool,
    pub evidence_matches_source: bool,
    pub identity_anomaly: Option<IdentityAnomaly>,
    pub split: DatasetSplit,
    pub duplicate_group: String,
    pub instruction_like: bool,
    pub article_file: Option<ArticleFile>,
    pub ambiguous_paths: Vec<String>,
    pub mapping_status: String,
    pub baseline: Baseline,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DatasetSplit {
    Dev,
    Heldout,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArticleFile {
    pub path: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub fetched_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_input_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Baseline {
    pub record_path: String,
    pub request_id: String,
    pub path_kind: PathKind,
    pub model_id: String,
    pub recorded_utc: String,
    pub priority: u8,
    pub category: String,
    pub tags: Vec<String>,
    pub rationale: String,
    pub usage: Usage,
    pub cost_microdollars: u64,
    pub wall_ms: u64,
    pub cache_status: String,
}

impl Manifest {
    pub fn calculate_hash(&self) -> anyhow::Result<String> {
        let mut without_hash = self.clone();
        without_hash.created_utc.clear();
        without_hash.manifest_hash.clear();
        Ok(sha256(&serde_json::to_string(&without_hash)?))
    }

    pub fn refresh_hash(&mut self) -> anyhow::Result<()> {
        self.manifest_hash = self.calculate_hash()?;
        Ok(())
    }
}

pub fn manifest_path(experiment_dir: &Path) -> PathBuf {
    experiment_dir.join("manifest.json")
}

pub fn load_manifest(path: &Path) -> anyhow::Result<Manifest> {
    let text =
        fs::read_to_string(path).with_context(|| format!("reading manifest {}", path.display()))?;
    let manifest: Manifest = serde_json::from_str(&text)
        .with_context(|| format!("parsing manifest {}", path.display()))?;
    if manifest.calculate_hash()? != manifest.manifest_hash {
        bail!("manifest hash mismatch in {}", path.display());
    }
    let root = path.parent().context("manifest has no parent directory")?;
    let rubric_path = root.join(&manifest.frozen_inputs.rubric_path);
    let rubric = fs::read_to_string(&rubric_path)
        .with_context(|| format!("reading frozen rubric {}", rubric_path.display()))?;
    if sha256(&rubric) != manifest.frozen_inputs.rubric_sha256 {
        bail!("frozen rubric hash mismatch: {}", rubric_path.display());
    }
    for article in &manifest.articles {
        let evidence_path = root.join(&article.text_path);
        let evidence = fs::read_to_string(&evidence_path).with_context(|| {
            format!(
                "reading frozen article {} ({})",
                article.article_id,
                evidence_path.display()
            )
        })?;
        if sha256(&evidence) != article.evidence_hash {
            bail!(
                "frozen article hash mismatch for {}: {}",
                article.article_id,
                evidence_path.display()
            );
        }
    }
    Ok(manifest)
}

pub fn write_manifest(experiment_dir: &Path, manifest: &Manifest) -> anyhow::Result<()> {
    fs::create_dir_all(experiment_dir)?;
    fs::write(
        manifest_path(experiment_dir),
        serde_json::to_vec_pretty(manifest)?,
    )?;
    Ok(())
}
