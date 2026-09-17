//! Read-only clean-text index for corpus article mapping.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::Utc;
use engine_logging::engine_warn;
use harvester_engine::{derive_clean_text, parse_frontmatter, TRUNCATION_MARKER};
use serde::{Deserialize, Serialize};

use crate::prompt_identity::sha256;

const INDEX_VERSION: u32 = 2;
const PREFIX_BYTES: usize = 4096;

/// A cached clean-text entry for one corpus markdown file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArticleIndexEntry {
    pub path: String,
    pub file_len: u64,
    pub file_mtime_ms: i128,
    pub clean_hash: String,
    pub prefix4k_hash: String,
    pub clean_len: usize,
    pub title: Option<String>,
    pub url: Option<String>,
    pub fetched_utc: Option<String>,
}

/// The persistent index cache format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArticleIndexCache {
    pub index_version: u32,
    pub built_utc: String,
    pub entries: Vec<ArticleIndexEntry>,
}

/// Work performed while building or loading an article index.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArticleIndexStats {
    pub reused: usize,
    pub derived: usize,
    pub skipped: usize,
}

/// In-memory lookup structures used by the article matcher.
#[derive(Debug, Clone)]
pub struct ArticleIndex {
    pub entries: Vec<ArticleIndexEntry>,
    pub stats: ArticleIndexStats,
    by_hash: HashMap<String, Vec<usize>>,
    by_prefix: HashMap<String, Vec<usize>>,
}

/// The outcome of mapping frozen evidence to archive files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingOutcome {
    MappedByHash(usize),
    MappedByPrefix(usize),
    Unmapped,
    Ambiguous(Vec<usize>),
}

impl MappingOutcome {
    pub fn status(&self) -> &'static str {
        match self {
            Self::MappedByHash(_) => "mapped_by_hash",
            Self::MappedByPrefix(_) => "mapped_by_prefix",
            Self::Unmapped => "unmapped",
            Self::Ambiguous(_) => "ambiguous",
        }
    }
}

impl ArticleIndex {
    pub fn from_entries(entries: Vec<ArticleIndexEntry>) -> Self {
        Self::from_entries_with_stats(entries, ArticleIndexStats::default())
    }

    fn from_entries_with_stats(entries: Vec<ArticleIndexEntry>, stats: ArticleIndexStats) -> Self {
        let mut by_hash: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_prefix: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, entry) in entries.iter().enumerate() {
            by_hash
                .entry(entry.clean_hash.clone())
                .or_default()
                .push(index);
            by_prefix
                .entry(entry.prefix4k_hash.clone())
                .or_default()
                .push(index);
        }
        Self {
            entries,
            stats,
            by_hash,
            by_prefix,
        }
    }
}

/// Applies the hash, truncated-prefix, unmapped mapping rule in that order.
///
/// Exact hash matches need no article read. Prefix candidates are re-derived
/// lazily so the index never retains the corpus clean text in memory.
pub fn match_article(
    source_content_hash: &str,
    frozen_text: &str,
    index: &ArticleIndex,
) -> anyhow::Result<MappingOutcome> {
    if let Some(matches) = index.by_hash.get(source_content_hash) {
        return Ok(unique_or_ambiguous(matches));
    }
    let Some(prefix) = frozen_text.strip_suffix(TRUNCATION_MARKER) else {
        return Ok(MappingOutcome::Unmapped);
    };
    let candidate_indexes = if prefix.len() >= PREFIX_BYTES {
        let bucket = sha256(first_bytes_at_char_boundary(prefix, PREFIX_BYTES));
        index.by_prefix.get(&bucket).cloned().unwrap_or_default()
    } else {
        // A very small content budget cannot fill a 4 KiB bucket. Preserve the
        // mapping contract by checking each file lazily.
        (0..index.entries.len()).collect()
    };
    let config = harvester_engine::eval_support::default_content_prep_config();
    let mut matches = Vec::new();
    for entry_index in candidate_indexes {
        let derived = derive_article(Path::new(&index.entries[entry_index].path), &config)?;
        if derived.clean_text.starts_with(prefix) {
            matches.push(entry_index);
        }
    }
    Ok(match matches.as_slice() {
        [] => MappingOutcome::Unmapped,
        [only] => MappingOutcome::MappedByPrefix(*only),
        many => MappingOutcome::Ambiguous(many.to_vec()),
    })
}

fn unique_or_ambiguous(matches: &[usize]) -> MappingOutcome {
    match matches {
        [] => MappingOutcome::Unmapped,
        [only] => MappingOutcome::MappedByHash(*only),
        many => MappingOutcome::Ambiguous(many.to_vec()),
    }
}

/// Builds (or refreshes) the on-disk article index cache without changing corpus files.
pub fn build_or_load_index(
    output_dir: &Path,
    linked_dir: Option<&Path>,
    cache_path: &Path,
) -> anyhow::Result<ArticleIndex> {
    let old = fs::read_to_string(cache_path)
        .ok()
        .and_then(|text| serde_json::from_str::<ArticleIndexCache>(&text).ok())
        .filter(|cache| cache.index_version == INDEX_VERSION);
    let old_by_path = old
        .as_ref()
        .map(|cache| {
            cache
                .entries
                .iter()
                .map(|entry| (entry.path.clone(), entry))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let (entries, stats) = build_entries(output_dir, linked_dir, &old_by_path)?;
    let serializable = ArticleIndexCache {
        index_version: INDEX_VERSION,
        built_utc: Utc::now().to_rfc3339(),
        entries: entries.clone(),
    };
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(cache_path, serde_json::to_vec_pretty(&serializable)?)?;
    Ok(ArticleIndex::from_entries_with_stats(entries, stats))
}

/// Builds a transient index without creating a cache. Used by dry-runs.
pub fn build_index(output_dir: &Path, linked_dir: Option<&Path>) -> anyhow::Result<ArticleIndex> {
    let (entries, stats) = build_entries(output_dir, linked_dir, &HashMap::new())?;
    Ok(ArticleIndex::from_entries_with_stats(entries, stats))
}

fn build_entries(
    output_dir: &Path,
    linked_dir: Option<&Path>,
    old_by_path: &HashMap<String, &ArticleIndexEntry>,
) -> anyhow::Result<(Vec<ArticleIndexEntry>, ArticleIndexStats)> {
    let mut paths = markdown_paths(output_dir)?;
    let default_linked = output_dir.join("linked");
    if let Some(dir) = linked_dir {
        paths.extend(markdown_paths(dir)?);
    } else if default_linked.is_dir() {
        paths.extend(markdown_paths(&default_linked)?);
    }
    paths.sort();
    let config = harvester_engine::eval_support::default_content_prep_config();
    let mut entries = Vec::with_capacity(paths.len());
    let mut stats = ArticleIndexStats::default();
    for path in paths {
        let metadata =
            fs::metadata(&path).with_context(|| format!("reading metadata {}", path.display()))?;
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |value| value.as_millis() as i128);
        let key = path.to_string_lossy().into_owned();
        if let Some(previous) = old_by_path.get(&key) {
            if previous.file_len == metadata.len() && previous.file_mtime_ms == mtime {
                entries.push((*previous).clone());
                stats.reused += 1;
                continue;
            }
        }
        match index_entry(&path, metadata.len(), mtime, &config) {
            Ok(entry) => {
                entries.push(entry);
                stats.derived += 1;
            }
            Err(error) => {
                stats.skipped += 1;
                engine_warn!("[harvester_eval] run_id=freeze article_id=unknown operation=index_article path={} error={error}", path.display());
            }
        }
    }
    Ok((entries, stats))
}

fn markdown_paths(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    Ok(fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|extension| extension.to_str()) == Some("md")
        })
        .collect())
}

fn index_entry(
    path: &Path,
    file_len: u64,
    file_mtime_ms: i128,
    config: &harvester_engine::ContentPrepConfig,
) -> anyhow::Result<ArticleIndexEntry> {
    let derived = derive_article(path, config)?;
    Ok(ArticleIndexEntry {
        path: path.to_string_lossy().into_owned(),
        file_len,
        file_mtime_ms,
        clean_hash: sha256(&derived.clean_text),
        prefix4k_hash: prefix_hash(&derived.clean_text),
        clean_len: derived.clean_text.len(),
        title: derived.title,
        url: derived.url,
        fetched_utc: derived.fetched_utc,
    })
}

struct DerivedArticle {
    clean_text: String,
    title: Option<String>,
    url: Option<String>,
    fetched_utc: Option<String>,
}

fn derive_article(
    path: &Path,
    config: &harvester_engine::ContentPrepConfig,
) -> anyhow::Result<DerivedArticle> {
    let markdown =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let frontmatter = parse_frontmatter(&markdown);
    let source_url = frontmatter
        .as_ref()
        .and_then(|value| value.url.as_deref())
        .map(str::trim)
        .unwrap_or("");
    let clean = derive_clean_text(
        &markdown,
        source_url,
        frontmatter
            .as_ref()
            .and_then(|value| value.title.as_deref()),
        config,
    );
    Ok(DerivedArticle {
        clean_text: clean.text().to_string(),
        title: frontmatter.as_ref().and_then(|value| value.title.clone()),
        url: frontmatter.as_ref().and_then(|value| value.url.clone()),
        fetched_utc: frontmatter.and_then(|value| value.fetched_utc),
    })
}

fn prefix_hash(text: &str) -> String {
    sha256(first_bytes_at_char_boundary(text, PREFIX_BYTES))
}

/// Returns no more than `max_bytes`, without splitting a UTF-8 code point.
fn first_bytes_at_char_boundary(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &Path, text: &str) -> ArticleIndexEntry {
        ArticleIndexEntry {
            path: path.to_string_lossy().into_owned(),
            file_len: text.len() as u64,
            file_mtime_ms: 0,
            clean_hash: sha256(text),
            prefix4k_hash: prefix_hash(text),
            clean_len: text.len(),
            title: None,
            url: None,
            fetched_utc: None,
        }
    }

    #[test]
    fn exact_hash_mapping_needs_no_article_read_and_reports_ambiguity() {
        let missing = Path::new("missing.md");
        let full = "a clean article that continues";
        let index = ArticleIndex::from_entries(vec![entry(missing, full)]);
        assert!(matches!(
            match_article(&sha256(full), "anything", &index).unwrap(),
            MappingOutcome::MappedByHash(0)
        ));
        let duplicate =
            ArticleIndex::from_entries(vec![entry(missing, full), entry(missing, full)]);
        assert!(
            matches!(match_article(&sha256(full), full, &duplicate).unwrap(), MappingOutcome::Ambiguous(paths) if paths.len() == 2)
        );
    }

    #[test]
    fn utf8_prefix_boundary_does_not_split_a_code_point() {
        let text = format!("{}é-tail", "a".repeat(PREFIX_BYTES - 1));
        let prefix = first_bytes_at_char_boundary(&text, PREFIX_BYTES);
        assert_eq!(prefix.len(), PREFIX_BYTES - 1);
        assert!(prefix.is_char_boundary(prefix.len()));
    }
}
