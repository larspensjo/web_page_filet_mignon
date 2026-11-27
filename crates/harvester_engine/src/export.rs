use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::{DateTime, SecondsFormat, TimeDelta, Timelike, Utc};
use engine_logging::{engine_info, engine_warn};

use crate::archive_url_key;
use crate::briefing::ArchiveReadHints;
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

/// Why an article is represented by another document in the archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFoldReason {
    IdenticalContent,
    SameEvent,
}

impl ArchiveFoldReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SameEvent => "same_event",
            Self::IdenticalContent => "identical_content",
        }
    }
}

/// Members are supplied in representative election order, not header display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveFoldedArticle {
    pub url: String,
    pub reason: ArchiveFoldReason,
    pub signal_key: Option<String>,
}

/// Canonical representative URL keys mapped to their folded articles.
pub type ArchiveFoldReport = BTreeMap<String, Vec<ArchiveFoldedArticle>>;

#[derive(serde::Serialize)]
struct FoldedHeader<'a> {
    reason: ArchiveFoldReason,
    url: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signal_key: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSummary {
    pub doc_count: usize,
    /// Number of listed files whose contents were read, including non-articles.
    pub files_read: usize,
    pub total_tokens: u64,
    pub output_path: PathBuf,
    pub window_count: Option<usize>,
    pub unexported_by_priority: Option<[usize; 6]>,
    /// Bounded withheld URL counts in same_event, identical_content order.
    pub unexported_folded: Option<[usize; 2]>,
}

/// A rendered archive whose target is untouched until the supplied writer publishes it.
pub struct PreparedArchive {
    trace: ExportTrace,
    summary: ExportSummary,
    markdown: String,
}

impl PreparedArchive {
    pub fn publish<T, E>(
        mut self,
        write: impl FnOnce(&Path, &str) -> Result<T, E>,
    ) -> Result<(ExportSummary, T), E> {
        self.trace.phase("write-archive");
        engine_info!(
            "[archive-export] bytes={} path={}",
            self.markdown.len(),
            self.trace.path.display()
        );
        let publication = write(&self.trace.path, &self.markdown)?;
        self.trace.completed = true;
        engine_info!(
            "[archive-export] completed docs={} bytes={} path={} elapsed_ms={}",
            self.summary.doc_count,
            self.markdown.len(),
            self.trace.path.display(),
            self.trace.started.elapsed().as_millis()
        );
        Ok((self.summary, publication))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveReadProgress {
    pub files_checked: usize,
    pub files_total: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveTarget<'a> {
    InOutput { basename: &'a str },
    External { path: &'a Path },
}

/// The exact fetch-time bounds of an archive request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveWindow {
    since_utc: Option<DateTime<Utc>>,
    until_utc: DateTime<Utc>,
}

impl ArchiveWindow {
    pub fn from_submission(since_utc: Option<DateTime<Utc>>, submitted_at: DateTime<Utc>) -> Self {
        let remainder = submitted_at.nanosecond() % 1_000_000_000;
        let until_utc = if remainder == 0 {
            submitted_at
        } else {
            submitted_at + TimeDelta::nanoseconds(i64::from(1_000_000_000 - remainder))
        };
        Self::from_bounds(since_utc, until_utc)
    }

    pub(crate) fn from_bounds(since_utc: Option<DateTime<Utc>>, until_utc: DateTime<Utc>) -> Self {
        Self {
            since_utc,
            until_utc,
        }
    }

    pub fn since_utc(self) -> Option<DateTime<Utc>> {
        self.since_utc
    }

    pub fn until_utc(self) -> DateTime<Utc> {
        self.until_utc
    }

    pub fn contains(self, fetched_utc: Option<&str>) -> bool {
        fetched_utc
            .and_then(|date| DateTime::parse_from_rfc3339(date).ok())
            .is_none_or(|date| {
                self.since_utc.is_none_or(|since| date >= since) && date < self.until_utc
            })
    }
}

pub fn format_archive_timestamp(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

pub fn is_safe_archive_basename(name: &str) -> bool {
    !name.contains(['/', '\\', '\0'])
        && Path::new(name)
            .file_name()
            .is_some_and(|basename| basename == name)
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("persist error: {0}")]
    Persist(#[from] PersistError),
    #[error("invalid archive basename: {0}")]
    InvalidBasename(String),
    #[error("frontmatter missing required fields in file {0}")]
    MissingFrontmatter(String),
}

#[derive(Debug, Default)]
pub(crate) struct DocMeta {
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) fetched_utc: String,
    token_count: Option<u32>,
    body: String,
    filename: String,
}

/// Records the last reached boundary even when an export returns early or unwinds.
struct ExportTrace {
    path: PathBuf,
    started: Instant,
    phase: &'static str,
    last_file: Option<PathBuf>,
    completed: bool,
}

impl ExportTrace {
    fn phase(&mut self, phase: &'static str) {
        self.phase = phase;
        self.last_file = None;
        engine_info!(
            "[archive-export] phase={} path={} elapsed_ms={}",
            phase,
            self.path.display(),
            self.started.elapsed().as_millis()
        );
    }
}

impl Drop for ExportTrace {
    fn drop(&mut self) {
        if !self.completed {
            engine_warn!(
                "[archive-export] interrupted phase={} path={} last_file={:?} panicking={} elapsed_ms={}",
                self.phase,
                self.path.display(),
                self.last_file,
                std::thread::panicking(),
                self.started.elapsed().as_millis()
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_triage_archive(
    output_dir: &Path,
    basename: &str,
    ordered_urls: &[String],
    window: ArchiveWindow,
    use_summaries: bool,
    summaries: &HashMap<String, String>,
    annotations: &HashMap<String, ArchiveDocAnnotations>,
    priority_snapshot: &HashMap<String, u8>,
    folds: Option<&ArchiveFoldReport>,
) -> Result<ExportSummary, ExportError> {
    build_triage_archive_with(
        output_dir,
        ArchiveTarget::InOutput { basename },
        ordered_urls,
        window,
        use_summaries,
        summaries,
        annotations,
        priority_snapshot,
        folds,
        None,
        &mut |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_triage_archive_with(
    output_dir: &Path,
    target: ArchiveTarget<'_>,
    ordered_urls: &[String],
    window: ArchiveWindow,
    use_summaries: bool,
    summaries: &HashMap<String, String>,
    annotations: &HashMap<String, ArchiveDocAnnotations>,
    priority_snapshot: &HashMap<String, u8>,
    folds: Option<&ArchiveFoldReport>,
    hints: Option<&ArchiveReadHints>,
    on_progress: &mut dyn FnMut(ArchiveReadProgress),
) -> Result<ExportSummary, ExportError> {
    let prepared = prepare_triage_archive_with(
        output_dir,
        target,
        ordered_urls,
        window,
        use_summaries,
        summaries,
        annotations,
        priority_snapshot,
        folds,
        hints,
        on_progress,
    )?;
    let (summary, ()) = prepared.publish(write_export_file)?;
    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_triage_archive_with(
    output_dir: &Path,
    target: ArchiveTarget<'_>,
    ordered_urls: &[String],
    window: ArchiveWindow,
    use_summaries: bool,
    summaries: &HashMap<String, String>,
    annotations: &HashMap<String, ArchiveDocAnnotations>,
    priority_snapshot: &HashMap<String, u8>,
    folds: Option<&ArchiveFoldReport>,
    hints: Option<&ArchiveReadHints>,
    on_progress: &mut dyn FnMut(ArchiveReadProgress),
) -> Result<PreparedArchive, ExportError> {
    let path = match target {
        ArchiveTarget::InOutput { basename } if is_safe_archive_basename(basename) => {
            output_dir.join(basename)
        }
        ArchiveTarget::InOutput { basename } => {
            return Err(ExportError::InvalidBasename(basename.to_owned()));
        }
        ArchiveTarget::External { path } => path.to_path_buf(),
    };
    let since_utc = window.since_utc();
    let mut trace = ExportTrace {
        path,
        started: Instant::now(),
        phase: "prepare-directory",
        last_file: None,
        completed: false,
    };
    trace.phase("prepare-directory");
    ensure_output_dir(output_dir)?;
    trace.phase("write-manifest");
    write_corpus_manifest(output_dir)?;
    trace.phase("collect-files");
    let entries = ordered_archive_window_paths(output_dir, &trace.path)?;
    let file_count = entries.len();
    engine_info!(
        "[archive-export] files={} path={}",
        file_count,
        trace.path.display()
    );

    trace.phase("read-articles");
    let mut docs_by_url: HashMap<String, DocMeta> = HashMap::new();
    let mut files_read = 0;
    let mut report_progress = |files_checked| {
        on_progress(ArchiveReadProgress {
            files_checked,
            files_total: file_count,
        })
    };
    report_progress(0);
    let cutoff_with_hints = since_utc.zip(hints);
    for (index, path) in entries.into_iter().enumerate() {
        trace.last_file = Some(path.clone());
        if index % 1_000 == 0 {
            engine_info!(
                "[archive-export] phase=read-articles scanned={} total={} current_file={} path={} elapsed_ms={}",
                index, file_count, path.display(), trace.path.display(), trace.started.elapsed().as_millis()
            );
        }
        let skip_read = cutoff_with_hints.is_some_and(|(since, hints)| {
            path.parent() == Some(output_dir) && hints.proves_before(&path, since)
        });
        if !skip_read {
            files_read += 1;
            if let Some((_, meta)) = read_archive_window_doc(output_dir, &path)? {
                if window.contains(Some(&meta.fetched_utc)) {
                    let normalized = archive_url_key(&meta.url);
                    docs_by_url.entry(normalized).or_insert(meta);
                }
            }
        }
        report_progress(index + 1);
    }

    trace.phase("select-documents");
    let window_count = docs_by_url.len();
    let window_titles: HashMap<_, _> = docs_by_url
        .iter()
        .map(|(url, doc)| (url.clone(), doc.title.clone()))
        .collect();
    let mut docs = Vec::new();
    let mut document_folds = Vec::new();
    let mut selected = HashSet::new();
    let mut requested = HashSet::new();
    for url in ordered_urls {
        let normalized = archive_url_key(url);
        if !requested.insert(normalized.clone()) || selected.contains(&normalized) {
            continue;
        }
        let members = folds.and_then(|report| report.get(&normalized));
        for member in members.into_iter().flatten() {
            if !window_titles.contains_key(&archive_url_key(&member.url)) {
                engine_warn!(
                    "[archive-export] folded article missing from window kept_url={} folded_url={}",
                    normalized,
                    member.url
                );
            }
        }
        let surviving_url = if docs_by_url.contains_key(&normalized) {
            Some(normalized.clone())
        } else {
            let promoted = members.and_then(|members| {
                members
                    .iter()
                    .map(|member| archive_url_key(&member.url))
                    .find(|key| docs_by_url.contains_key(key) && !selected.contains(key))
            });
            if let Some(promoted) = &promoted {
                engine_warn!(
                    "[archive-export] promoting folded article lost_url={} promoted_url={}",
                    normalized,
                    promoted
                );
            } else if folds.is_some() {
                engine_warn!(
                    "[archive-export] no surviving article for selected url={}",
                    normalized
                );
            }
            promoted
        };
        if let Some(key) = surviving_url {
            selected.insert(key.clone());
            docs.push(
                docs_by_url
                    .remove(&key)
                    .expect("surviving document is in window"),
            );
            document_folds.push(
                members
                    .into_iter()
                    .flatten()
                    .filter(|member| archive_url_key(&member.url) != key)
                    .collect::<Vec<_>>(),
            );
        }
    }
    // Count only listed URLs still withheld after every selected document is removed.
    let mut counted_folded = HashSet::new();
    let mut unexported_folded = [0; 2];
    for member in document_folds.iter().flatten() {
        let key = archive_url_key(&member.url);
        if docs_by_url.contains_key(&key) && counted_folded.insert(key) {
            unexported_folded[match member.reason {
                ArchiveFoldReason::SameEvent => 0,
                ArchiveFoldReason::IdenticalContent => 1,
            }] += 1;
        }
    }
    let unexported_by_priority = if since_utc.is_some() {
        unexported_priority_counts(&docs_by_url, priority_snapshot)
    } else {
        [0; 6]
    };

    engine_info!(
        "[archive-export] selected={} requested={} window_count={} summaries={} path={}",
        docs.len(),
        ordered_urls.len(),
        window_count,
        summaries.len(),
        trace.path.display()
    );
    trace.phase("render-documents");
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
        if folds.is_some() {
            write_folded_header(&mut buffer, &document_folds[position], &window_titles);
        }
        buffer.push('\n');
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
        window,
        window_count,
        unexported_by_priority,
        folds.map(|_| unexported_folded),
    );

    let summary = ExportSummary {
        doc_count: docs.len(),
        files_read,
        total_tokens,
        output_path: trace.path.clone(),
        window_count: since_utc.is_some().then_some(window_count),
        unexported_by_priority: since_utc.is_some().then_some(unexported_by_priority),
        unexported_folded: (since_utc.is_some() && folds.is_some()).then_some(unexported_folded),
    };
    Ok(PreparedArchive {
        trace,
        summary,
        markdown: buffer,
    })
}

/// Shared artifact exclusions and stable filename precedence for archive readers.
/// Only names are checked here; `read_archive_window_doc` skips exports found by content,
/// so each article is read once per scan.
pub(crate) fn ordered_archive_window_paths(
    output_dir: &Path,
    target_path: &Path,
) -> Result<Vec<PathBuf>, ExportError> {
    let mut entries = collect_archive_md_files(output_dir)?;
    entries.retain(|path| path != target_path && !is_named_archive_artifact(path, output_dir));
    entries.sort_by_key(|path| path.file_name().map(|name| name.to_os_string()));
    Ok(entries)
}

fn collect_archive_md_files(output_dir: &Path) -> Result<Vec<PathBuf>, ExportError> {
    let mut entries = collect_md_files(output_dir)?;
    let linked_dir = output_dir.join("linked");
    if linked_dir.exists() {
        entries.extend(collect_md_files(&linked_dir)?);
    }
    Ok(entries)
}

fn is_named_archive_artifact(path: &Path, output_dir: &Path) -> bool {
    if path.parent() != Some(output_dir) {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name == "archive.md" || (name.starts_with("archive-") && name.ends_with(".md"))
}

/// A renamed export in the output root is recognised by its leading archive marker.
fn is_archive_content(path: &Path, output_dir: &Path, bytes: &[u8]) -> bool {
    path.parent() == Some(output_dir)
        && (bytes.starts_with(DOC_START_MARKER.as_bytes())
            || bytes.starts_with(ARCHIVE_INDEX_MARKER.as_bytes()))
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

fn write_export_file(target: &Path, content: &str) -> Result<(), ExportError> {
    let filename = target.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "archive target must have a filename",
        )
    })?;
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    AtomicFileWriter::new(parent.to_path_buf()).write(filename, content)?;
    Ok(())
}

pub(crate) fn fetched_before_checkpoint(fetched_utc: &str, since: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(fetched_utc).is_ok_and(|parsed| parsed.with_timezone(&Utc) < since)
}

/// Shared archive population: unreadable article metadata is logged and excluded;
/// archive exports are skipped silently.
pub(crate) fn read_archive_window_doc(
    output_dir: &Path,
    path: &Path,
) -> Result<Option<(String, DocMeta)>, ExportError> {
    let relative = path
        .strip_prefix(output_dir)
        .unwrap_or(path)
        .to_string_lossy();
    let bytes = fs::read(path)?;
    if is_archive_content(path, output_dir, &bytes) {
        return Ok(None);
    }
    let content = String::from_utf8(bytes)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    match parse_doc(&content, &relative) {
        Ok(meta) => Ok(Some((content, meta))),
        Err(error) => {
            engine_warn!(
                "[archive-window] skipping article path={} reason={}",
                path.display(),
                error
            );
            Ok(None)
        }
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
}

fn write_folded_header(
    buffer: &mut String,
    members: &[&ArchiveFoldedArticle],
    titles: &HashMap<String, String>,
) {
    use std::fmt::Write;
    let mut entries: Vec<_> = members
        .iter()
        .map(|member| {
            let title = titles
                .get(&archive_url_key(&member.url))
                .map(|title| sanitize_scalar(title));
            FoldedHeader {
                reason: member.reason,
                url: &member.url,
                title,
                signal_key: member.signal_key.as_deref(),
            }
        })
        .collect();
    entries.sort_by(|a, b| a.reason.cmp(&b.reason).then(a.url.cmp(b.url)));
    let json = serde_json::to_string(&entries).expect("folded header strings serialize as JSON");
    let _ = writeln!(buffer, "folded: {json}");
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

#[allow(clippy::too_many_arguments)]
fn write_archive_index(
    buffer: &mut String,
    rows: &[(usize, usize, &DocMeta, Option<&ArchiveDocAnnotations>)],
    parsed_fetched: &[(DateTime<Utc>, String)],
    window: ArchiveWindow,
    window_count: usize,
    unexported_by_priority: [usize; 6],
    unexported_folded: Option<[usize; 2]>,
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
    if window.since_utc().is_some() {
        let _ = writeln!(buffer, "window_count: {window_count}");
        let [priority_5, priority_4, priority_3, priority_2, priority_1, unavailable] =
            unexported_by_priority;
        let _ = writeln!(
            buffer,
            "unexported_by_priority: {{\"5\":{priority_5},\"4\":{priority_4},\"3\":{priority_3},\"2\":{priority_2},\"1\":{priority_1},\"unavailable\":{unavailable}}}"
        );
        if let Some([same_event, identical_content]) = unexported_folded {
            let _ = writeln!(buffer, "unexported_folded: {{\"same_event\":{same_event},\"identical_content\":{identical_content}}}");
        }
    }
    if let Some(since) = window.since_utc() {
        let _ = writeln!(
            buffer,
            "window_since_utc: {}",
            format_archive_timestamp(since)
        );
    }
    let _ = writeln!(
        buffer,
        "window_until_utc: {}",
        format_archive_timestamp(window.until_utc())
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folded_title_sanitizes_every_scalar_separator_before_json_encoding() {
        // LF cannot reach a parsed title through the line-based frontmatter reader.
        let member = ArchiveFoldedArticle {
            url: "https://example.com/member".into(),
            reason: ArchiveFoldReason::SameEvent,
            signal_key: None,
        };
        let titles = [(
            archive_url_key(&member.url),
            "=====start\rcr\nlf\u{0085}nel\u{2028}ls\u{2029}ps,\"".into(),
        )]
        .into_iter()
        .collect();
        let mut buffer = String::new();
        write_folded_header(&mut buffer, &[&member], &titles);
        assert_eq!(buffer.lines().count(), 1);
        let json: serde_json::Value =
            serde_json::from_str(buffer.trim_end().strip_prefix("folded: ").unwrap()).unwrap();
        assert_eq!(json[0]["title"], " =====startcrlfnellsps,\"");
    }
}
