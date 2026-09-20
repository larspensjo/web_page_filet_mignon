use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use engine_logging::engine_warn;
use serde_json::json;

use crate::archive_url_key;
use crate::corpus_manifest::write_corpus_manifest;
use crate::frontmatter::{parse_frontmatter, strip_frontmatter};
use crate::persist::{ensure_output_dir, AtomicFileWriter, PersistError};
use crate::truncate_to_char_boundary;

/// Maximum character count for full-article fallback bodies in summary mode.
pub const MAX_FALLBACK_BODY_CHARS: usize = 50_000;

const DOC_START_MARKER: &str = "===== DOC START =====";
const DOC_END_MARKER: &str = "===== DOC END =====";
const ARCHIVE_INDEX_MARKER: &str = "===== ARCHIVE INDEX =====";
const INDEX_END_MARKER: &str = "===== INDEX END =====";
const RESERVED_MARKERS: [&str; 4] = [
    DOC_START_MARKER,
    DOC_END_MARKER,
    ARCHIVE_INDEX_MARKER,
    INDEX_END_MARKER,
];

/// Per-document judgments supplied by the reducer at archive submission time.
/// All fields are optional because scoring and triage may not have completed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveDocAnnotations {
    pub priority: Option<u8>,
    pub tags: Option<Vec<String>>,
    pub triage_model: Option<String>,
    pub signal_key: Option<String>,
    pub signal_score: Option<u8>,
    pub themes: Option<Vec<String>>,
}

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
    pub window_count: Option<usize>,
    pub unexported_by_priority: Option<[usize; 6]>,
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
    write_corpus_manifest(output_dir)?;
    let mut entries = collect_archive_md_files(output_dir)?;
    exclude_export_artifacts(
        &mut entries,
        output_dir,
        &options.output_filename,
        &options.manifest_filename,
    );
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
        let normalized = archive_url_key(&meta.url);
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
    let manifest_path =
        write_manifest(output_dir, &options.manifest_filename, &docs, total_tokens)?;

    Ok(ExportSummary {
        doc_count: docs.len(),
        total_tokens,
        output_path,
        manifest_path,
        window_count: None,
        unexported_by_priority: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn build_triage_archive(
    output_dir: &Path,
    basename: &str,
    ordered_urls: &[String],
    since_utc: Option<DateTime<Utc>>,
    options: ExportOptions,
    use_summaries: bool,
    summaries: &HashMap<String, String>,
    annotations: &HashMap<String, ArchiveDocAnnotations>,
    priority_snapshot: &HashMap<String, u8>,
) -> Result<ExportSummary, ExportError> {
    ensure_output_dir(output_dir)?;
    write_corpus_manifest(output_dir)?;
    let mut entries = collect_archive_md_files(output_dir)?;
    exclude_export_artifacts(
        &mut entries,
        output_dir,
        basename,
        &options.manifest_filename,
    );
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
        let normalized = archive_url_key(&meta.url);
        docs_by_url.entry(normalized).or_insert(meta);
    }

    let window_count = docs_by_url.len();
    let mut docs = Vec::new();
    let mut selected = HashSet::new();
    for url in ordered_urls {
        let normalized = archive_url_key(url);
        if !selected.insert(normalized.clone()) {
            continue;
        }
        if let Some(meta) = docs_by_url.remove(&normalized) {
            docs.push(meta);
        }
    }
    let unexported_by_priority = if since_utc.is_some() {
        unexported_priority_counts(&docs_by_url, priority_snapshot)
    } else {
        [0; 6]
    };

    let mut buffer = String::new();
    let mut index_rows = Vec::new();
    let mut parsed_fetched = Vec::new();
    let mut total_tokens: u64 = 0;
    let mut lines_written = 0usize;
    for (position, doc) in docs.iter().enumerate() {
        if let Some(t) = doc.token_count {
            total_tokens += t as u64;
        }
        let line = lines_written + 1;
        let block_start = buffer.len();
        buffer.push_str(DOC_START_MARKER);
        buffer.push('\n');
        let normalized = archive_url_key(&doc.url);
        let annotation = annotations.get(&normalized);
        let (content_label, body) = if !use_summaries {
            ("full", doc.body.trim_end().to_string())
        } else if let Some(summary_body) = summaries.get(&normalized) {
            ("summary", summary_body.trim_end().to_string())
        } else {
            let full_body = doc.body.trim_end();
            let truncated = truncate_to_char_boundary(full_body, MAX_FALLBACK_BODY_CHARS);
            let content_label = if full_body.chars().count() > MAX_FALLBACK_BODY_CHARS {
                "full-truncated"
            } else {
                "full"
            };
            (content_label, truncated.to_string())
        };
        write_archive_header(&mut buffer, doc, position + 1, content_label, annotation);
        buffer.push_str(&escape_archive_body(&body));
        buffer.push('\n');
        buffer.push_str(DOC_END_MARKER);
        buffer.push_str("\n\n");
        lines_written += buffer[block_start..]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
        if let Ok(parsed) = DateTime::parse_from_rfc3339(&doc.fetched_utc) {
            parsed_fetched.push((parsed.with_timezone(&Utc), doc.fetched_utc.clone()));
        }
        index_rows.push((position + 1, line, doc, annotation));
    }
    write_archive_index(
        &mut buffer,
        &index_rows,
        &parsed_fetched,
        since_utc.is_some(),
        window_count,
        unexported_by_priority,
    );

    let output_path = write_export_file(output_dir, basename, &buffer)?;
    let manifest_path =
        write_manifest(output_dir, &options.manifest_filename, &docs, total_tokens)?;

    Ok(ExportSummary {
        doc_count: docs.len(),
        total_tokens,
        output_path,
        manifest_path,
        window_count: since_utc.is_some().then_some(window_count),
        unexported_by_priority: since_utc.is_some().then_some(unexported_by_priority),
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

fn exclude_export_artifacts(
    entries: &mut Vec<PathBuf>,
    output_dir: &Path,
    output_filename: &str,
    manifest_filename: &Option<String>,
) {
    let output_artifact = output_dir.join(output_filename);
    let manifest_artifact = manifest_filename.as_ref().map(|name| output_dir.join(name));

    entries.retain(|path| {
        if is_archive_artifact(path, output_dir, output_filename) {
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

fn is_archive_artifact(path: &Path, output_dir: &Path, output_filename: &str) -> bool {
    if path.parent() != Some(output_dir) {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name == output_filename || name == "archive.md" {
        return true;
    }
    if name.starts_with("archive-") && name.ends_with(".md") {
        return true;
    }
    fs::read(path)
        .map(|bytes| {
            bytes.starts_with(DOC_START_MARKER.as_bytes())
                || bytes.starts_with(ARCHIVE_INDEX_MARKER.as_bytes())
        })
        .unwrap_or(false)
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
        filename: filename.to_string(),
    })
}

fn write_archive_header(
    buffer: &mut String,
    doc: &DocMeta,
    position: usize,
    content: &str,
    annotations: Option<&ArchiveDocAnnotations>,
) {
    use std::fmt::Write;
    let _ = writeln!(buffer, "url: {}", sanitize_scalar(&doc.url));
    let _ = writeln!(buffer, "title: {}", sanitize_scalar(&doc.title));
    let _ = writeln!(buffer, "tokens: {}", doc.token_count.unwrap_or(0));
    let _ = writeln!(buffer, "fetched_utc: {}", sanitize_scalar(&doc.fetched_utc));
    let _ = writeln!(buffer, "filename: {}", sanitize_scalar(&doc.filename));
    let _ = writeln!(buffer, "content: {content}");
    buffer.push_str("export_schema: 2\n");
    let _ = writeln!(buffer, "doc: {position}");
    if let Some(annotations) = annotations {
        if let Some(priority) = annotations.priority {
            let _ = writeln!(buffer, "priority: {priority}");
        }
        if let Some(tags) = &annotations.tags {
            let _ = writeln!(buffer, "tags: {}", json_list(tags));
        }
        if let Some(model) = &annotations.triage_model {
            let _ = writeln!(buffer, "triage_model: {}", sanitize_scalar(model));
        }
        if let Some(signal_key) = &annotations.signal_key {
            let _ = writeln!(buffer, "signal_key: {}", sanitize_scalar(signal_key));
        }
        if let Some(score) = annotations.signal_score {
            let _ = writeln!(buffer, "signal_score: {score}");
        }
        if let Some(themes) = &annotations.themes {
            let _ = writeln!(buffer, "themes: {}", json_list(themes));
        }
    }
    buffer.push('\n');
}

fn json_list(values: &[String]) -> String {
    let mut sorted = values.to_vec();
    sorted.sort();
    serde_json::to_string(&sorted).expect("strings always serialize as JSON")
}

fn sanitize_scalar(value: &str) -> String {
    let mut sanitized = value.replace(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}'], "");
    if sanitized.starts_with("=====") {
        sanitized.insert(0, ' ');
    }
    sanitized
}

fn escape_archive_body(body: &str) -> String {
    body.replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(|line| {
            let stripped = line.trim_start_matches('\\').trim_end();
            if RESERVED_MARKERS.contains(&stripped) {
                format!("\\{line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_archive_index(
    buffer: &mut String,
    rows: &[(usize, usize, &DocMeta, Option<&ArchiveDocAnnotations>)],
    parsed_fetched: &[(DateTime<Utc>, String)],
    include_coverage: bool,
    window_count: usize,
    unexported_by_priority: [usize; 6],
) {
    use std::fmt::Write;
    let fetched_from = parsed_fetched
        .iter()
        .min_by_key(|(date, _)| *date)
        .map(|(_, original)| original.as_str())
        .unwrap_or("-");
    let fetched_to = parsed_fetched
        .iter()
        .max_by_key(|(date, _)| *date)
        .map(|(_, original)| original.as_str())
        .unwrap_or("-");
    buffer.push_str(ARCHIVE_INDEX_MARKER);
    buffer.push_str("\nexport_schema: 2\n");
    let _ = writeln!(buffer, "doc_count: {}", rows.len());
    let _ = writeln!(buffer, "fetched_from: {fetched_from}");
    let _ = writeln!(buffer, "fetched_to: {fetched_to}");
    if include_coverage {
        let _ = writeln!(buffer, "window_count: {window_count}");
        let [priority_5, priority_4, priority_3, priority_2, priority_1, unavailable] =
            unexported_by_priority;
        let _ = writeln!(
            buffer,
            "unexported_by_priority: {{\"5\":{priority_5},\"4\":{priority_4},\"3\":{priority_3},\"2\":{priority_2},\"1\":{priority_1},\"unavailable\":{unavailable}}}"
        );
    }
    buffer.push_str("doc | line | fetched_utc | priority | signal_key | title\n");
    for (doc, line, meta, annotation) in rows {
        let priority = annotation
            .and_then(|annotation| annotation.priority)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string());
        let signal_key = annotation
            .and_then(|annotation| annotation.signal_key.as_deref())
            .map(sanitize_scalar)
            .unwrap_or_else(|| "-".to_string());
        let title = sanitize_scalar(&meta.title).replace('|', "/");
        let fetched_utc = sanitize_scalar(&meta.fetched_utc).replace('|', "/");
        let _ = writeln!(
            buffer,
            "{doc} | {line} | {fetched_utc} | {priority} | {signal_key} | {title}"
        );
    }
    buffer.push_str(INDEX_END_MARKER);
    buffer.push('\n');
}

fn unexported_priority_counts(
    docs_by_url: &HashMap<String, DocMeta>,
    priority_snapshot: &HashMap<String, u8>,
) -> [usize; 6] {
    let mut counts = [0; 6];
    for url in docs_by_url.keys() {
        match priority_snapshot.get(url).copied() {
            Some(5) => counts[0] += 1,
            Some(4) => counts[1] += 1,
            Some(3) => counts[2] += 1,
            Some(2) => counts[3] += 1,
            Some(1) => counts[4] += 1,
            Some(priority) => {
                engine_warn!(
                    "[archive-export] out-of-range triage priority url={} priority={}",
                    url,
                    priority
                );
                counts[5] += 1;
            }
            None => counts[5] += 1,
        }
    }
    counts
}
