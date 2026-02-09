use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

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
    filename: String,
}

pub fn build_concatenated_export(
    output_dir: &Path,
    options: ExportOptions,
) -> Result<ExportSummary, ExportError> {
    ensure_output_dir(output_dir)?;
    let mut entries = collect_md_files(output_dir)?;
    let linked_dir = output_dir.join("linked");
    if linked_dir.exists() {
        entries.extend(collect_md_files(&linked_dir)?);
    }
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

    let writer = AtomicFileWriter::new(output_dir.to_path_buf());
    let output_path = writer.write(&options.output_filename, &buffer)?;

    let manifest_path = if let Some(name) = options.manifest_filename {
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
        let path = writer.write(&name, &manifest.to_string())?;
        Some(path)
    } else {
        None
    };

    Ok(ExportSummary {
        doc_count: docs.len(),
        total_tokens,
        output_path,
        manifest_path,
    })
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
        filename: filename.to_string(),
    })
}
