use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::json;

use crate::frontmatter::{parse_frontmatter, strip_frontmatter};
use crate::persist::{ensure_output_dir, AtomicFileWriter, PersistError};
use url::Url;

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub output_filename: String,
    pub manifest_filename: Option<String>,
    pub delimiter_start: String,
    pub delimiter_end: String,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            output_filename: "export.txt".to_string(),
            manifest_filename: Some("manifest.json".to_string()),
            delimiter_start: "===== DOC START =====".to_string(),
            delimiter_end: "===== DOC END =====".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSummary {
    pub doc_count: usize,
    pub total_tokens: u64,
    pub output_path: PathBuf,
    pub manifest_path: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("persist error: {0}")]
    Persist(#[from] PersistError),
    #[error("frontmatter missing required fields in file {0}")]
    MissingFrontmatter(String),
}

#[derive(Debug, Default)]
struct DocMeta {
    url: String,
    title: String,
    fetched_utc: String,
    token_count: Option<u32>,
    body: String,
    raw_content: String,
    filename: String,
}

pub fn build_concatenated_export(
    output_dir: &Path,
    options: ExportOptions,
) -> Result<ExportSummary, ExportError> {
    ensure_output_dir(output_dir)?;
    let mut entries = collect_archive_md_files(output_dir)?;
    exclude_export_artifacts(&mut entries, output_dir, &options);
    entries.sort_by_key(|path| path.file_name().map(|name| name.to_os_string()));

    let mut docs = Vec::new();
    let mut seen = HashSet::new();
    for path in entries {
        let relative = path
            .strip_prefix(output_dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let content = fs::read_to_string(&path)?;
        let meta = parse_doc(&content, &relative)?;
        let normalized = normalize_url(&meta.url);
        if seen.insert(normalized) {
            docs.push(meta);
        }
    }

    let mut buffer = String::new();
    let mut total_tokens: u64 = 0;
    for doc in &docs {
        if let Some(t) = doc.token_count {
            total_tokens += t as u64;
        }
        buffer.push_str(&options.delimiter_start);
        buffer.push('\n');
        buffer.push_str(&format!(
            "url: {}\ntitle: {}\ntokens: {}\nfetched_utc: {}\nfilename: {}\n\n",
            doc.url,
            doc.title,
            doc.token_count.unwrap_or(0),
            doc.fetched_utc,
            doc.filename
        ));
        buffer.push_str(doc.body.trim_end());
        buffer.push('\n');
        buffer.push_str(&options.delimiter_end);
        buffer.push_str("\n\n");
    }

    let output_path = write_export_file(output_dir, &options.output_filename, &buffer)?;
    let manifest_path = write_manifest(output_dir, &options.manifest_filename, &docs, total_tokens)?;

    Ok(ExportSummary {
        doc_count: docs.len(),
        total_tokens,
        output_path,
        manifest_path,
    })
}

pub fn build_triage_archive(
    output_dir: &Path,
    ordered_urls: &[String],
    since_utc: Option<DateTime<Utc>>,
    options: ExportOptions,
) -> Result<ExportSummary, ExportError> {
    ensure_output_dir(output_dir)?;
    let mut entries = collect_archive_md_files(output_dir)?;
    exclude_export_artifacts(&mut entries, output_dir, &options);
    entries.sort_by_key(|path| path.file_name().map(|name| name.to_os_string()));

    let mut docs_by_url: HashMap<String, DocMeta> = HashMap::new();
    for path in entries {
        let relative = path
            .strip_prefix(output_dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let content = fs::read_to_string(&path)?;
        let meta = parse_doc(&content, &relative)?;
        if !passes_since_filter(&meta, since_utc) {
            continue;
        }
        let normalized = normalize_url(&meta.url);
        docs_by_url.entry(normalized).or_insert(meta);
    }

    let mut docs = Vec::new();
    let mut selected = HashSet::new();
    for url in ordered_urls {
        let normalized = normalize_url(url);
        if !selected.insert(normalized.clone()) {
            continue;
        }
        if let Some(meta) = docs_by_url.remove(&normalized) {
            docs.push(meta);
        }
    }

    let mut buffer = String::new();
    let mut total_tokens: u64 = 0;
    for doc in &docs {
        if let Some(t) = doc.token_count {
            total_tokens += t as u64;
        }
        buffer.push_str(&options.delimiter_start);
        buffer.push('\n');
        buffer.push_str(&doc.raw_content);
        if !doc.raw_content.ends_with('\n') {
            buffer.push('\n');
        }
        buffer.push_str(&options.delimiter_end);
        buffer.push_str("\n\n");
    }

    let archive_filename = archive_filename_for_range(since_utc, latest_fetched_utc(&docs));
    let output_path = write_export_file(output_dir, &archive_filename, &buffer)?;
    let manifest_path = write_manifest(output_dir, &options.manifest_filename, &docs, total_tokens)?;

    Ok(ExportSummary {
        doc_count: docs.len(),
        total_tokens,
        output_path,
        manifest_path,
    })
}

fn collect_archive_md_files(output_dir: &Path) -> Result<Vec<PathBuf>, ExportError> {
    let mut entries = collect_md_files(output_dir)?;
    let linked_dir = output_dir.join("linked");
    if linked_dir.exists() {
        entries.extend(collect_md_files(&linked_dir)?);
    }
    Ok(entries)
}

fn exclude_export_artifacts(entries: &mut Vec<PathBuf>, output_dir: &Path, options: &ExportOptions) {
    let output_artifact = output_dir.join(&options.output_filename);
    let manifest_artifact = options
        .manifest_filename
        .as_ref()
        .map(|name| output_dir.join(name));

    entries.retain(|path| {
        if is_archive_artifact(path, output_dir) {
            return false;
        }
        if *path == output_artifact {
            return false;
        }
        if let Some(manifest) = manifest_artifact.as_ref() {
            if path == manifest {
                return false;
            }
        }
        true
    });
}

fn is_archive_artifact(path: &Path, output_dir: &Path) -> bool {
    if path.parent() != Some(output_dir) {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name == "archive.md" || (name.starts_with("archive-") && name.ends_with(".md"))
}

fn archive_filename_for_range(
    since_utc: Option<DateTime<Utc>>,
    latest_fetched_utc: Option<DateTime<Utc>>,
) -> String {
    let from = since_utc
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "all".to_string());
    let to = latest_fetched_utc
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| from.clone());
    format!("archive-{from}-{to}.md")
}

fn latest_fetched_utc(docs: &[DocMeta]) -> Option<DateTime<Utc>> {
    docs.iter()
        .filter_map(|doc| DateTime::parse_from_rfc3339(&doc.fetched_utc).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .max()
}

fn collect_md_files(dir: &Path) -> Result<Vec<PathBuf>, ExportError> {
    let mut entries = Vec::new();
    if dir.exists() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false)
                && entry.path().extension().and_then(|s| s.to_str()) == Some("md")
            {
                entries.push(entry.path());
            }
        }
    }
    Ok(entries)
}

fn normalize_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    if let Ok(mut parsed) = Url::parse(trimmed) {
        parsed.set_fragment(None);
        if let Some(port) = parsed.port() {
            let normalized_port = match (parsed.scheme(), port) {
                ("http", 80) | ("https", 443) => None,
                _ => Some(port),
            };
            let _ = parsed.set_port(normalized_port);
        }
        return parsed.into();
    }
    trimmed.to_lowercase()
}

fn write_export_file(
    output_dir: &Path,
    output_filename: &str,
    content: &str,
) -> Result<PathBuf, ExportError> {
    let writer = AtomicFileWriter::new(output_dir.to_path_buf());
    Ok(writer.write(output_filename, content)?)
}

fn write_manifest(
    output_dir: &Path,
    manifest_filename: &Option<String>,
    docs: &[DocMeta],
    total_tokens: u64,
) -> Result<Option<PathBuf>, ExportError> {
    if let Some(name) = manifest_filename {
        let manifest = json!({
            "doc_count": docs.len(),
            "total_tokens": total_tokens,
            "files": docs.iter().map(|d| {
                json!({
                    "filename": d.filename,
                    "title": d.title,
                    "url": d.url,
                    "tokens": d.token_count.unwrap_or(0),
                    "fetched_utc": d.fetched_utc
                })
            }).collect::<Vec<_>>()
        });
        let writer = AtomicFileWriter::new(output_dir.to_path_buf());
        let path = writer.write(name, &manifest.to_string())?;
        Ok(Some(path))
    } else {
        Ok(None)
    }
}

fn passes_since_filter(meta: &DocMeta, since_utc: Option<DateTime<Utc>>) -> bool {
    let Some(since_dt) = since_utc else {
        return true;
    };
    match DateTime::parse_from_rfc3339(&meta.fetched_utc) {
        Ok(parsed) => parsed.with_timezone(&Utc) >= since_dt,
        Err(_) => true,
    }
}

fn parse_doc(content: &str, filename: &str) -> Result<DocMeta, ExportError> {
    let fields = parse_frontmatter(content)
        .ok_or_else(|| ExportError::MissingFrontmatter(filename.to_string()))?;
    let url = fields.url.clone().unwrap_or_default();
    let title = fields.title.clone().unwrap_or_default();
    let fetched = fields.fetched_utc.clone().unwrap_or_default();
    if url.is_empty() || title.is_empty() || fetched.is_empty() {
        return Err(ExportError::MissingFrontmatter(filename.to_string()));
    }
    let body = strip_frontmatter(content).to_string();
    Ok(DocMeta {
        url,
        title,
        fetched_utc: fetched,
        token_count: fields.token_count,
        body,
        raw_content: content.to_string(),
        filename: filename.to_string(),
    })
}
