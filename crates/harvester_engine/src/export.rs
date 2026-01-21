use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::persist::{AtomicFileWriter, PersistError};

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
    let mut entries: Vec<_> = fs::read_dir(output_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut docs = Vec::new();
    for entry in entries {
        let path = entry.path();
        let content = fs::read_to_string(&path)?;
        let meta = parse_doc(&content, entry.file_name().to_string_lossy().as_ref())?;
        docs.push(meta);
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

fn parse_doc(content: &str, filename: &str) -> Result<DocMeta, ExportError> {
    let mut lines = content.lines();
    if lines.next() != Some("---") {
        return Err(ExportError::MissingFrontmatter(filename.to_string()));
    }
    let mut meta = DocMeta {
        filename: filename.to_string(),
        ..Default::default()
    };
    for line in &mut lines {
        if line.trim() == "---" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim();
            match key {
                "url" => meta.url = val.to_string(),
                "title" => meta.title = val.to_string(),
                "fetched_utc" => meta.fetched_utc = val.to_string(),
                "token_count" => meta.token_count = val.parse::<u32>().ok(),
                _ => {}
            }
        }
    }
    let body: String = lines.collect::<Vec<_>>().join("\n");
    meta.body = body;
    if meta.url.is_empty() || meta.title.is_empty() || meta.fetched_utc.is_empty() {
        return Err(ExportError::MissingFrontmatter(filename.to_string()));
    }
    Ok(meta)
}
