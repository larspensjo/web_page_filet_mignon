//! Browser-saved webpage importer.
//!
//! Scans a flat directory of `.htm`/`.html` files saved from a browser and
//! imports them into the normal archive format without network access.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use scraper::{Html, Selector};
use serde_json::Value;

use crate::content_extraction::{ExtractionPipeline, ExtractionPolicy};
use crate::content_prep::{
    derive_clean_text, BoilerplatePolicy, ContentPrepConfig, NormalizationPolicy,
};
use crate::decode::decode_html;
use crate::filename::{import_filename_base, resolve_non_overwriting_filename};
use crate::frontmatter::{build_imported_markdown_document, ImportedFrontmatterFields};
use crate::persist::AtomicFileWriter;
use crate::token::WhitespaceTokenCounter;
use crate::write_corpus_manifest;

/// Maximum accepted file size before decode/DOM parse.
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// Minimum normalized clean text length in Unicode scalar values.
const MIN_CLEAN_TEXT_CHARS: usize = 200;

/// How many bytes of raw file content to scan for the browser saved-from comment.
const BROWSER_COMMENT_SCAN_BYTES: usize = 2048;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single candidate file found in a saved-webpage directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedWebpageFile {
    pub source_path: PathBuf,
    pub basename: String,
    pub file_size_bytes: u64,
}

/// Result of scanning a directory for saved webpage candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedWebpageScanResult {
    pub candidates: Vec<SavedWebpageFile>,
    pub ignored_directories: usize,
    pub ignored_non_html_files: usize,
}

/// A successfully extracted saved webpage, before persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedDocument {
    pub canonical_url: String,
    pub title: Option<String>,
    pub encoding: String,
    pub fetched_utc: String,
    pub published_utc: Option<String>,
    pub markdown_body: String,
    pub clean_text: String,
    pub content_hash: String,
    pub warnings: Vec<String>,
}

/// A reference to an imported archive entry after persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedArchiveRef {
    pub persisted_path: PathBuf,
    pub canonical_url: String,
    pub content_hash: String,
    pub fetched_utc: String,
}

/// The pipeline stage at which an import failure occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportFailureStage {
    FileRead,
    OversizeRejection,
    Decode,
    CanonicalUrlExtraction,
    ContentExtraction,
    QualityThreshold,
    Persistence,
}

/// A per-file import failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportFailure {
    pub source_path: PathBuf,
    pub stage: ImportFailureStage,
    pub reason: String,
}

/// Aggregate result of a batch import operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub scanned_count: usize,
    pub imported_entries: Vec<ImportedArchiveRef>,
    pub warnings: Vec<String>,
    pub failures: Vec<ImportFailure>,
    pub duplicate_url_count: usize,
    pub duplicate_content_count: usize,
}

/// Options for a batch import run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportOptions {
    /// Directory where imported markdown files will be written.
    pub archive_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Scan `dir` for top-level `.htm`/`.html` files.
///
/// Does not recurse into subdirectories. Candidates are sorted deterministically
/// by path. `dir` is canonicalized before scanning.
pub fn scan_saved_webpage_dir(dir: &Path) -> Result<SavedWebpageScanResult, String> {
    let canonical_dir = dir
        .canonicalize()
        .map_err(|e| format!("failed to canonicalize {}: {}", dir.display(), e))?;

    let read_dir = fs::read_dir(&canonical_dir).map_err(|e| {
        format!(
            "failed to list directory {}: {}",
            canonical_dir.display(),
            e
        )
    })?;

    let mut candidates = Vec::new();
    let mut ignored_directories = 0usize;
    let mut ignored_non_html_files = 0usize;

    for entry in read_dir {
        let entry = entry.map_err(|e| format!("failed to read dir entry: {e}"))?;
        let path = entry.path();
        let metadata = entry
            .metadata()
            .map_err(|e| format!("failed to get metadata for {}: {e}", path.display()))?;

        if metadata.is_dir() {
            ignored_directories += 1;
            continue;
        }

        let is_html = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("htm") || ext.eq_ignore_ascii_case("html"))
            .unwrap_or(false);

        if !is_html {
            ignored_non_html_files += 1;
            continue;
        }

        let basename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        candidates.push(SavedWebpageFile {
            source_path: path,
            basename,
            file_size_bytes: metadata.len(),
        });
    }

    candidates.sort_by(|a, b| a.source_path.cmp(&b.source_path));

    Ok(SavedWebpageScanResult {
        candidates,
        ignored_directories,
        ignored_non_html_files,
    })
}

/// Import all `.htm`/`.html` files found in `dir`, persisting them to the
/// archive directory in `options`.
///
/// Returns an `ImportReport` regardless of per-file failures. A run with zero
/// persisted entries is captured via `failures`; callers decide the exit
/// classification.
pub fn import_saved_webpages(dir: &Path, options: &ImportOptions) -> ImportReport {
    let scan_result = match scan_saved_webpage_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            return ImportReport {
                scanned_count: 0,
                imported_entries: Vec::new(),
                warnings: Vec::new(),
                failures: vec![ImportFailure {
                    source_path: dir.to_path_buf(),
                    stage: ImportFailureStage::FileRead,
                    reason: e,
                }],
                duplicate_url_count: 0,
                duplicate_content_count: 0,
            };
        }
    };

    engine_info!(
        "[import-saved-web] scan requested dir={} candidates={}",
        dir.display(),
        scan_result.candidates.len()
    );

    let scanned_count = scan_result.candidates.len();

    // Snapshot existing archive URLs and hashes once for duplicate detection.
    let existing_urls = load_existing_archive_urls(&options.archive_dir);
    let existing_hashes = load_existing_archive_hashes(&options.archive_dir);

    let now = Utc::now();
    let fetched_utc = now.to_rfc3339();
    let timestamp_str = format_import_timestamp(&now);

    let writer = AtomicFileWriter::new(options.archive_dir.clone());
    let token_counter = WhitespaceTokenCounter;

    let mut imported_entries: Vec<ImportedArchiveRef> = Vec::new();
    let mut all_warnings: Vec<String> = Vec::new();
    let mut failures: Vec<ImportFailure> = Vec::new();
    let mut duplicate_url_count = 0usize;
    let mut duplicate_content_count = 0usize;

    if let Err(err) = write_corpus_manifest(&options.archive_dir) {
        let reason = format!("failed to write corpus manifest: {err}");
        engine_warn!("[import-saved-web] {}", reason);
        failures.push(ImportFailure {
            source_path: options.archive_dir.clone(),
            stage: ImportFailureStage::Persistence,
            reason,
        });
    }

    // Track URLs and hashes seen within this batch.
    let mut batch_urls: HashSet<String> = HashSet::new();
    let mut batch_hashes: HashSet<String> = HashSet::new();

    for candidate in &scan_result.candidates {
        match extract_webpage_content(&candidate.source_path) {
            Err(failure) => {
                engine_warn!(
                    "[import-saved-web] failed file={} stage={:?} reason={}",
                    candidate.basename,
                    failure.stage,
                    failure.reason
                );
                failures.push(failure);
            }
            Ok((doc, mut warnings)) => {
                // Duplicate URL check.
                if existing_urls.contains(&doc.canonical_url)
                    || batch_urls.contains(&doc.canonical_url)
                {
                    let w = format!("duplicate-url url={}", doc.canonical_url);
                    engine_warn!("[import-saved-web] {}", w);
                    warnings.push(w.clone());
                    all_warnings.push(w);
                    duplicate_url_count += 1;
                }
                batch_urls.insert(doc.canonical_url.clone());

                // Duplicate content hash check.
                if existing_hashes.contains(&doc.content_hash)
                    || batch_hashes.contains(&doc.content_hash)
                {
                    let w = format!(
                        "duplicate-content hash={}",
                        doc.content_hash.get(..8).unwrap_or(&doc.content_hash)
                    );
                    engine_warn!("[import-saved-web] {}", w);
                    warnings.push(w.clone());
                    all_warnings.push(w);
                    duplicate_content_count += 1;
                }
                batch_hashes.insert(doc.content_hash.clone());

                // Build full archive markdown.
                let imported_fields = ImportedFrontmatterFields {
                    imported_utc: fetched_utc.clone(),
                    published_utc: doc.published_utc.clone(),
                    source_path_hint: Some(candidate.basename.clone()),
                };
                let (_, markdown_doc) = build_imported_markdown_document(
                    &doc.canonical_url,
                    doc.title.as_deref(),
                    &doc.encoding,
                    &fetched_utc,
                    &doc.markdown_body,
                    &imported_fields,
                    &token_counter,
                );

                // Resolve non-overwriting filename.
                let base_name =
                    import_filename_base(doc.title.as_deref(), &doc.canonical_url, &timestamp_str);
                let filename = resolve_non_overwriting_filename(&options.archive_dir, &base_name);

                // Persist.
                match writer.write(&filename, &markdown_doc) {
                    Err(e) => {
                        let reason = format!("failed to write {filename}: {e}");
                        engine_warn!(
                            "[import-saved-web] failed file={} stage=Persistence reason={}",
                            candidate.basename,
                            reason
                        );
                        failures.push(ImportFailure {
                            source_path: candidate.source_path.clone(),
                            stage: ImportFailureStage::Persistence,
                            reason,
                        });
                    }
                    Ok(persisted_path) => {
                        if let Err(err) = write_corpus_manifest(&options.archive_dir) {
                            let reason = format!("failed to write corpus manifest: {err}");
                            engine_warn!("[import-saved-web] {}", reason);
                            failures.push(ImportFailure {
                                source_path: options.archive_dir.clone(),
                                stage: ImportFailureStage::Persistence,
                                reason,
                            });
                        }
                        engine_info!(
                            "[import-saved-web] imported url={} path={}",
                            doc.canonical_url,
                            persisted_path.display()
                        );
                        imported_entries.push(ImportedArchiveRef {
                            persisted_path,
                            canonical_url: doc.canonical_url,
                            content_hash: doc.content_hash,
                            fetched_utc: fetched_utc.clone(),
                        });
                    }
                }
            }
        }
    }

    engine_info!(
        "[import-saved-web] complete imported={} failed={} duplicate_urls={} duplicate_content={}",
        imported_entries.len(),
        failures.len(),
        duplicate_url_count,
        duplicate_content_count
    );

    ImportReport {
        scanned_count,
        imported_entries,
        warnings: all_warnings,
        failures,
        duplicate_url_count,
        duplicate_content_count,
    }
}

/// Import a single saved webpage file.
/// Generates its own `fetched_utc` timestamp from the current time.
pub fn import_single_saved_webpage(
    path: &Path,
    options: &ImportOptions,
) -> Result<ImportedDocument, ImportFailure> {
    let (mut doc, _warnings) = extract_webpage_content(path)?;

    let now = Utc::now();
    doc.fetched_utc = now.to_rfc3339();

    // Persist if archive_dir is set.
    if !options.archive_dir.as_os_str().is_empty() {
        let timestamp_str = format_import_timestamp(&now);
        let base_name =
            import_filename_base(doc.title.as_deref(), &doc.canonical_url, &timestamp_str);
        let filename = resolve_non_overwriting_filename(&options.archive_dir, &base_name);
        let token_counter = WhitespaceTokenCounter;
        let imported_fields = ImportedFrontmatterFields {
            imported_utc: doc.fetched_utc.clone(),
            published_utc: doc.published_utc.clone(),
            source_path_hint: path
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string),
        };
        let (_, markdown_doc) = build_imported_markdown_document(
            &doc.canonical_url,
            doc.title.as_deref(),
            &doc.encoding,
            &doc.fetched_utc,
            &doc.markdown_body,
            &imported_fields,
            &token_counter,
        );
        let writer = AtomicFileWriter::new(options.archive_dir.clone());
        writer
            .write(&filename, &markdown_doc)
            .map_err(|e| ImportFailure {
                source_path: path.to_path_buf(),
                stage: ImportFailureStage::Persistence,
                reason: format!("failed to write {filename}: {e}"),
            })?;
        write_corpus_manifest(&options.archive_dir).map_err(|e| ImportFailure {
            source_path: path.to_path_buf(),
            stage: ImportFailureStage::Persistence,
            reason: format!("failed to write corpus manifest: {e}"),
        })?;
    }

    Ok(doc)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn extract_webpage_content(path: &Path) -> Result<(ImportedDocument, Vec<String>), ImportFailure> {
    // 1. Pre-flight size guard.
    let metadata = fs::metadata(path).map_err(|e| ImportFailure {
        source_path: path.to_path_buf(),
        stage: ImportFailureStage::FileRead,
        reason: format!("cannot read metadata: {e}"),
    })?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(ImportFailure {
            source_path: path.to_path_buf(),
            stage: ImportFailureStage::OversizeRejection,
            reason: format!(
                "file size {} bytes exceeds {MAX_FILE_BYTES} byte limit",
                metadata.len()
            ),
        });
    }

    // 2. Read raw bytes.
    let raw_bytes = fs::read(path).map_err(|e| ImportFailure {
        source_path: path.to_path_buf(),
        stage: ImportFailureStage::FileRead,
        reason: format!("cannot read file: {e}"),
    })?;

    // 3. Decode HTML.
    let decoded = decode_html(&raw_bytes, None).map_err(|e| ImportFailure {
        source_path: path.to_path_buf(),
        stage: ImportFailureStage::Decode,
        reason: format!("decode failure: {e}"),
    })?;
    let html = &decoded.html;
    let encoding = decoded.encoding_label;

    // 4. Extract metadata.
    let doc_tree = Html::parse_document(html);
    let mut warnings: Vec<String> = Vec::new();

    let canonical_url =
        extract_canonical_url(&doc_tree, &raw_bytes).ok_or_else(|| ImportFailure {
            source_path: path.to_path_buf(),
            stage: ImportFailureStage::CanonicalUrlExtraction,
            reason: "canonical URL not recoverable after all fallbacks".to_string(),
        })?;

    let title = extract_title(&doc_tree, &mut warnings);
    let published_utc = extract_published_utc(&doc_tree);

    // 5. Extract article body and convert to markdown.
    let pipeline = ExtractionPipeline::new(ExtractionPolicy::default());
    let extracted_article = pipeline.extract(html, Some(&canonical_url));
    let markdown_body = extracted_article.markdown;

    // 6. Derive clean text and validate quality threshold.
    let config = ContentPrepConfig {
        normalization: NormalizationPolicy::default(),
        boilerplate: BoilerplatePolicy::default(),
        token_counter: Arc::new(WhitespaceTokenCounter),
    };
    let clean = derive_clean_text(&markdown_body, &canonical_url, title.as_deref(), &config);
    let char_count = clean.text().chars().count();
    if char_count < MIN_CLEAN_TEXT_CHARS {
        return Err(ImportFailure {
            source_path: path.to_path_buf(),
            stage: ImportFailureStage::QualityThreshold,
            reason: format!(
                "normalized clean text has {char_count} chars, minimum is {MIN_CLEAN_TEXT_CHARS}"
            ),
        });
    }

    let doc = ImportedDocument {
        canonical_url,
        title,
        encoding,
        fetched_utc: String::new(), // filled by callers
        published_utc,
        markdown_body,
        clean_text: clean.text().to_string(),
        content_hash: clean.content_hash().to_string(),
        warnings: warnings.clone(),
    };

    Ok((doc, warnings))
}

/// Try to extract a canonical URL from the parsed HTML document.
///
/// Precedence:
/// 1. `<link rel="canonical" href="…">`
/// 2. `<meta property="og:url" content="…">`
/// 3. JSON-LD `url` or `mainEntityOfPage`
/// 4. Browser `<!-- saved from url=(N)URL -->` comment in the raw file prefix
fn extract_canonical_url(doc: &Html, raw_bytes: &[u8]) -> Option<String> {
    // 1. link[rel=canonical]
    if let Some(url) = try_link_canonical(doc) {
        return Some(url);
    }

    // 2. meta[property="og:url"]
    if let Some(url) = try_og_url(doc) {
        return Some(url);
    }

    // 3. JSON-LD
    if let Some(url) = try_json_ld_url(doc) {
        return Some(url);
    }

    // 4. Browser comment fallback
    browser_saved_url(raw_bytes)
}

fn try_link_canonical(doc: &Html) -> Option<String> {
    let sel = Selector::parse(r#"link[rel="canonical"]"#).ok()?;
    doc.select(&sel)
        .next()
        .and_then(|el| el.value().attr("href"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn try_og_url(doc: &Html) -> Option<String> {
    let sel = Selector::parse(r#"meta[property="og:url"]"#).ok()?;
    doc.select(&sel)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn try_json_ld_url(doc: &Html) -> Option<String> {
    let sel = Selector::parse(r#"script[type="application/ld+json"]"#).ok()?;
    for script in doc.select(&sel) {
        let text = script.text().collect::<String>();
        if let Ok(json) = serde_json::from_str::<Value>(&text) {
            if let Some(url) = json_ld_url_field(&json) {
                return Some(url);
            }
        }
    }
    None
}

fn json_ld_url_field(json: &Value) -> Option<String> {
    // Handle both single objects and arrays.
    let items: Vec<&Value> = match json {
        Value::Array(arr) => arr.iter().collect(),
        _ => vec![json],
    };
    for item in items {
        // Direct `url` field.
        if let Some(Value::String(u)) = item.get("url") {
            let trimmed = u.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
        // `mainEntityOfPage` — can be a string or object with `@id`.
        if let Some(mep) = item.get("mainEntityOfPage") {
            let candidate = match mep {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Object(obj) => obj
                    .get("@id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string()),
                _ => None,
            };
            if let Some(u) = candidate.filter(|s| !s.is_empty()) {
                return Some(u);
            }
        }
    }
    None
}

/// Scan the first `BROWSER_COMMENT_SCAN_BYTES` bytes for an IE/Edge
/// "saved from url" comment: `<!-- saved from url=(NNNN)URL -->`
fn browser_saved_url(raw_bytes: &[u8]) -> Option<String> {
    let prefix = &raw_bytes[..raw_bytes.len().min(BROWSER_COMMENT_SCAN_BYTES)];
    let text = std::str::from_utf8(prefix).ok()?;
    // Look for the marker.
    let marker = "saved from url=(";
    let start = text.find(marker)?;
    let after_marker = &text[start + marker.len()..];
    // The number in parentheses is the byte length of the URL — skip past it.
    let paren_close = after_marker.find(')')?;
    let url = after_marker[paren_close + 1..].trim();
    // The URL ends at the next whitespace or `-->`
    let end = url
        .find(|c: char| c.is_ascii_whitespace() || c == '-')
        .unwrap_or(url.len());
    let url = &url[..end];
    if url.is_empty() {
        None
    } else {
        Some(url.to_string())
    }
}

/// Extract the best available title.
///
/// Precedence:
/// 1. `<meta property="og:title">`
/// 2. JSON-LD `headline`
/// 3. `<title>`
fn extract_title(doc: &Html, warnings: &mut Vec<String>) -> Option<String> {
    let og = try_og_title(doc);
    let jld = try_json_ld_headline(doc);
    let tag = try_title_tag(doc);

    // Warn on disagreement between present sources.
    let candidates: Vec<(&str, &str)> = [
        ("og:title", og.as_deref()),
        ("json-ld headline", jld.as_deref()),
        ("<title>", tag.as_deref()),
    ]
    .into_iter()
    .filter_map(|(label, v)| v.map(|s| (label, s)))
    .collect();

    if candidates.len() > 1 {
        let first = candidates[0].1;
        if candidates.iter().any(|(_, v)| *v != first) {
            warnings.push(format!(
                "title disagreement: {}",
                candidates
                    .iter()
                    .map(|(l, v)| format!("{l}={v:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    let result = og.or(jld).or(tag);
    if result.is_none() {
        warnings.push("title missing".to_string());
    }
    result
}

fn try_og_title(doc: &Html) -> Option<String> {
    let sel = Selector::parse(r#"meta[property="og:title"]"#).ok()?;
    doc.select(&sel)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn try_json_ld_headline(doc: &Html) -> Option<String> {
    let sel = Selector::parse(r#"script[type="application/ld+json"]"#).ok()?;
    for script in doc.select(&sel) {
        let text = script.text().collect::<String>();
        if let Ok(json) = serde_json::from_str::<Value>(&text) {
            let items: Vec<&Value> = match &json {
                Value::Array(arr) => arr.iter().collect(),
                _ => vec![&json],
            };
            for item in items {
                if let Some(Value::String(h)) = item.get("headline") {
                    let trimmed = h.trim().to_string();
                    if !trimmed.is_empty() {
                        return Some(trimmed);
                    }
                }
            }
        }
    }
    None
}

fn try_title_tag(doc: &Html) -> Option<String> {
    let sel = Selector::parse("title").ok()?;
    doc.select(&sel)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Extract published timestamp from JSON-LD `datePublished`.
fn extract_published_utc(doc: &Html) -> Option<String> {
    let sel = Selector::parse(r#"script[type="application/ld+json"]"#).ok()?;
    for script in doc.select(&sel) {
        let text = script.text().collect::<String>();
        if let Ok(json) = serde_json::from_str::<Value>(&text) {
            let items: Vec<&Value> = match &json {
                Value::Array(arr) => arr.iter().collect(),
                _ => vec![&json],
            };
            for item in items {
                if let Some(Value::String(dp)) = item.get("datePublished") {
                    let trimmed = dp.trim().to_string();
                    if !trimmed.is_empty() {
                        return Some(trimmed);
                    }
                }
            }
        }
    }
    None
}

/// Format an import timestamp as `YYYYMMDDTHHMMSSfffZ`.
fn format_import_timestamp(dt: &chrono::DateTime<Utc>) -> String {
    format!(
        "{}{}Z",
        dt.format("%Y%m%dT%H%M%S"),
        dt.timestamp_subsec_millis()
            .to_string()
            .chars()
            .take(3)
            .collect::<String>()
            .chars()
            .collect::<String>(),
    )
}

/// Scan the archive directory once and collect all canonical URLs.
fn load_existing_archive_urls(archive_dir: &Path) -> HashSet<String> {
    let mut set = HashSet::new();
    let Ok(read_dir) = fs::read_dir(archive_dir) else {
        return set;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("md"))
            != Some(true)
        {
            continue;
        }
        if let Ok(content) = fs::read_to_string(&path) {
            if let Some(fields) = crate::frontmatter::parse_frontmatter(&content) {
                if let Some(url) = fields.url.filter(|u| !u.is_empty()) {
                    set.insert(url);
                }
            }
        }
    }
    set
}

/// Scan the archive directory once and collect all content hashes.
fn load_existing_archive_hashes(archive_dir: &Path) -> HashSet<String> {
    let config = ContentPrepConfig {
        normalization: NormalizationPolicy::default(),
        boilerplate: BoilerplatePolicy::default(),
        token_counter: Arc::new(WhitespaceTokenCounter),
    };

    let mut set = HashSet::new();
    let Ok(read_dir) = fs::read_dir(archive_dir) else {
        return set;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("md"))
            != Some(true)
        {
            continue;
        }
        if let Ok(content) = fs::read_to_string(&path) {
            if let Some(fields) = crate::frontmatter::parse_frontmatter(&content) {
                let url = fields.url.as_deref().unwrap_or("");
                let clean = derive_clean_text(&content, url, fields.title.as_deref(), &config);
                set.insert(clean.content_hash().to_string());
            }
        }
    }
    set
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_html(body: &str) -> String {
        format!(
            r#"<!DOCTYPE html><html><head><title>Test</title></head><body>{body}</body></html>"#
        )
    }

    fn make_html_with_meta(meta: &str, body: &str) -> String {
        format!(
            r#"<!DOCTYPE html><html><head>{meta}<title>Test</title></head><body>{body}</body></html>"#
        )
    }

    fn long_body(n: usize) -> String {
        "word ".repeat(n)
    }

    // --- scan tests ---

    #[test]
    fn scan_ignores_directories_and_non_html() {
        let dir = TempDir::new().unwrap();
        // Create an html file
        fs::write(dir.path().join("article.html"), b"content").unwrap();
        // Create a non-html file
        fs::write(dir.path().join("resource.css"), b"css").unwrap();
        // Create a subdirectory
        fs::create_dir(dir.path().join("article_files")).unwrap();

        let result = scan_saved_webpage_dir(dir.path()).unwrap();
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].basename, "article.html");
        assert_eq!(result.ignored_directories, 1);
        assert_eq!(result.ignored_non_html_files, 1);
    }

    #[test]
    fn scan_accepts_htm_extension() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("page.htm"), b"content").unwrap();
        let result = scan_saved_webpage_dir(dir.path()).unwrap();
        assert_eq!(result.candidates.len(), 1);
    }

    #[test]
    fn scan_ordering_is_deterministic() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("b_page.html"), b"b").unwrap();
        fs::write(dir.path().join("a_page.html"), b"a").unwrap();
        fs::write(dir.path().join("c_page.html"), b"c").unwrap();
        let result = scan_saved_webpage_dir(dir.path()).unwrap();
        let names: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.basename.as_str())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "candidates must be sorted by path");
    }

    // --- oversize pre-flight ---

    #[test]
    fn oversize_file_fails_before_html_parse() {
        let dir = TempDir::new().unwrap();
        let archive_dir = TempDir::new().unwrap();
        let path = dir.path().join("big.html");
        // Write a file that pretends to be > 5 MiB by creating metadata that reports size.
        // We fake this by writing exactly MAX_FILE_BYTES + 1 bytes.
        let big = vec![b'x'; (MAX_FILE_BYTES + 1) as usize];
        fs::write(&path, &big).unwrap();
        let opts = ImportOptions {
            archive_dir: archive_dir.path().to_path_buf(),
        };
        let result = import_single_saved_webpage(&path, &opts);
        assert!(result.is_err());
        let f = result.unwrap_err();
        assert_eq!(f.stage, ImportFailureStage::OversizeRejection);
    }

    // --- canonical URL extraction ---

    #[test]
    fn canonical_url_from_link_rel_canonical() {
        let html = make_html_with_meta(
            r#"<link rel="canonical" href="https://example.com/story"/>"#,
            &long_body(60),
        );
        let doc = Html::parse_document(&html);
        let url = extract_canonical_url(&doc, html.as_bytes());
        assert_eq!(url.as_deref(), Some("https://example.com/story"));
    }

    #[test]
    fn canonical_url_from_og_url_fallback() {
        let html = make_html_with_meta(
            r#"<meta property="og:url" content="https://og.example.com/page"/>"#,
            &long_body(60),
        );
        let doc = Html::parse_document(&html);
        let url = extract_canonical_url(&doc, html.as_bytes());
        assert_eq!(url.as_deref(), Some("https://og.example.com/page"));
    }

    #[test]
    fn canonical_url_from_json_ld() {
        let meta = r#"<script type="application/ld+json">{"@type":"Article","url":"https://ld.example.com/a"}</script>"#;
        let html = make_html_with_meta(meta, &long_body(60));
        let doc = Html::parse_document(&html);
        let url = extract_canonical_url(&doc, html.as_bytes());
        assert_eq!(url.as_deref(), Some("https://ld.example.com/a"));
    }

    #[test]
    fn canonical_url_from_browser_comment_fallback() {
        let raw = b"<!-- saved from url=(0038)https://browser.example.com/page --><html></html>";
        let doc = Html::parse_document(std::str::from_utf8(raw).unwrap());
        let url = extract_canonical_url(&doc, raw);
        assert_eq!(url.as_deref(), Some("https://browser.example.com/page"));
    }

    #[test]
    fn missing_canonical_url_returns_none() {
        let html = make_html(&long_body(60));
        let doc = Html::parse_document(&html);
        let url = extract_canonical_url(&doc, html.as_bytes());
        assert!(url.is_none());
    }

    // --- quality threshold ---

    #[test]
    fn content_shorter_than_threshold_fails() {
        let dir = TempDir::new().unwrap();
        let archive_dir = TempDir::new().unwrap();
        let html = make_html_with_meta(
            r#"<link rel="canonical" href="https://example.com/short"/>"#,
            "Too short.",
        );
        let path = dir.path().join("short.html");
        fs::write(&path, html.as_bytes()).unwrap();
        let opts = ImportOptions {
            archive_dir: archive_dir.path().to_path_buf(),
        };
        let result = import_single_saved_webpage(&path, &opts);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().stage,
            ImportFailureStage::QualityThreshold
        );
    }

    #[test]
    fn sufficient_content_is_imported_successfully() {
        let dir = TempDir::new().unwrap();
        let archive_dir = TempDir::new().unwrap();
        let html = make_html_with_meta(
            r#"<link rel="canonical" href="https://example.com/article"/>
               <meta property="og:title" content="Test Article"/>"#,
            &long_body(60),
        );
        let path = dir.path().join("article.html");
        fs::write(&path, html.as_bytes()).unwrap();
        let opts = ImportOptions {
            archive_dir: archive_dir.path().to_path_buf(),
        };
        let doc = import_single_saved_webpage(&path, &opts).unwrap();
        assert_eq!(doc.canonical_url, "https://example.com/article");
        assert_eq!(doc.title.as_deref(), Some("Test Article"));
    }

    // --- duplicate URL keeps both files ---

    #[test]
    fn duplicate_url_persists_second_archive_file_without_overwrite() {
        let dir = TempDir::new().unwrap();
        let archive_dir = TempDir::new().unwrap();
        let html = make_html_with_meta(
            r#"<link rel="canonical" href="https://dup.example.com/article"/>
               <meta property="og:title" content="Dup Article"/>"#,
            &long_body(60),
        );
        // Write two files with same canonical URL.
        let p1 = dir.path().join("a.html");
        let p2 = dir.path().join("b.html");
        fs::write(&p1, html.as_bytes()).unwrap();
        fs::write(&p2, html.as_bytes()).unwrap();

        let opts = ImportOptions {
            archive_dir: archive_dir.path().to_path_buf(),
        };
        let report = import_saved_webpages(dir.path(), &opts);

        // Both should be persisted (different filenames due to collision resolution).
        assert_eq!(report.imported_entries.len(), 2);
        assert_eq!(report.duplicate_url_count, 1);

        // Both archive files must exist.
        for entry in &report.imported_entries {
            assert!(
                entry.persisted_path.exists(),
                "expected {} to exist",
                entry.persisted_path.display()
            );
        }

        // The two archive file paths must differ.
        assert_ne!(
            report.imported_entries[0].persisted_path,
            report.imported_entries[1].persisted_path
        );
    }

    // --- imported markdown is parseable by existing readers ---

    #[test]
    fn imported_markdown_is_parseable_by_existing_readers() {
        let dir = TempDir::new().unwrap();
        let archive_dir = TempDir::new().unwrap();
        let html = make_html_with_meta(
            r#"<link rel="canonical" href="https://example.com/parseable"/>
               <meta property="og:title" content="Parseable"/>"#,
            &long_body(60),
        );
        let path = dir.path().join("parseable.html");
        fs::write(&path, html.as_bytes()).unwrap();
        let opts = ImportOptions {
            archive_dir: archive_dir.path().to_path_buf(),
        };
        import_single_saved_webpage(&path, &opts).unwrap();

        // Scan archive and find the imported file.
        let mut md_files: Vec<_> = fs::read_dir(archive_dir.path())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("md"))
                    == Some(true)
            })
            .collect();
        assert_eq!(md_files.len(), 1);
        let md_path = md_files.remove(0).path();
        let content = fs::read_to_string(&md_path).unwrap();
        let fields = crate::frontmatter::parse_frontmatter(&content).expect("frontmatter missing");
        assert_eq!(fields.url.as_deref(), Some("https://example.com/parseable"));
    }

    // --- path-based loading ---

    #[test]
    fn path_based_loading_preserves_same_url_duplicate_entries() {
        use crate::briefing::load_and_prepare_articles_by_path;
        use crate::llm::PromptRegistry;

        let dir = TempDir::new().unwrap();
        let archive_dir = TempDir::new().unwrap();
        let html = make_html_with_meta(
            r#"<link rel="canonical" href="https://dup2.example.com/story"/>
               <meta property="og:title" content="Dup Story"/>"#,
            &long_body(80),
        );
        let p1 = dir.path().join("x.html");
        let p2 = dir.path().join("y.html");
        fs::write(&p1, html.as_bytes()).unwrap();
        // Vary body slightly for second entry so content_hash differs.
        let html2 = make_html_with_meta(
            r#"<link rel="canonical" href="https://dup2.example.com/story"/>
               <meta property="og:title" content="Dup Story"/>"#,
            &format!("{} extra", long_body(80)),
        );
        fs::write(&p2, html2.as_bytes()).unwrap();

        let opts = ImportOptions {
            archive_dir: archive_dir.path().to_path_buf(),
        };
        let report = import_saved_webpages(dir.path(), &opts);
        assert_eq!(report.imported_entries.len(), 2);

        let paths: Vec<PathBuf> = report
            .imported_entries
            .iter()
            .map(|r| r.persisted_path.clone())
            .collect();

        let registry = PromptRegistry::with_defaults();
        let (articles, _) =
            load_and_prepare_articles_by_path(&paths, 256 * 1024, &registry).unwrap();

        // Both entries should load (same URL, different paths).
        assert_eq!(articles.len(), 2);
    }

    // --- format timestamp ---

    #[test]
    fn format_import_timestamp_is_reasonable() {
        let now = Utc::now();
        let ts = format_import_timestamp(&now);
        // e.g. "20260307T143022123Z"
        assert!(ts.ends_with('Z'), "must end with Z");
        assert!(ts.len() >= 16, "must have at least date+time chars");
    }
}
