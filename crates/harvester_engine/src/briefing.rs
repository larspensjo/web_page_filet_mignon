use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    thread,
};

use engine_logging::{engine_debug, engine_warn};
use url::Url;

use crate::content_prep::{
    derive_clean_text, truncate_to_budget, BoilerplatePolicy, CleanText, ContentPrepConfig,
    NormalizationPolicy,
};
use crate::frontmatter::parse_frontmatter;
use crate::llm::{PromptId, PromptRegistry};
use crate::token::WhitespaceTokenCounter;

fn parse_rfc3339_utc(label: &str, value: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    let snippet = if value.len() > 50 {
        &value[..50]
    } else {
        value
    };
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|e| format!("[briefing-filter] {label}: invalid RFC3339 '{snippet}': {e}"))
}

mod corpus_index;
pub use corpus_index::{
    CorpusScanIndex, CorpusScanStats, HeldArticle, TriageArticleDelta, WindowArticle,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LoadedArticle {
    pub url: String,
    pub source_title: Option<String>,
    pub prepared_text: String,
    pub content_hash: String,
    /// RFC3339 UTC timestamp from the article's frontmatter; `None` if absent or unparseable.
    pub fetched_utc: Option<String>,
}

/// Lightweight article metadata for archive scanning without content prep budgeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveArticleMeta {
    pub url: String,
    /// RFC3339 UTC timestamp from the article's frontmatter; `None` if absent or unparseable.
    pub fetched_utc: Option<String>,
    /// SHA256 content hash (derived from normalized clean text). `None` if derivation fails.
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArticleScanProgress {
    pub files_scanned: usize,
    pub files_total: usize,
}

struct ArticlePackage {
    url: String,
    source_title: Option<String>,
    clean_text: CleanText,
    /// RFC3339 UTC timestamp from frontmatter, preserved for downstream consumers.
    fetched_utc: Option<String>,
}

struct ArticleScanOutcome {
    index: usize,
    package: Option<ArticlePackage>,
    missing_fetched_utc: bool,
    malformed_fetched_utc: bool,
}

fn build_content_prep_config() -> ContentPrepConfig {
    ContentPrepConfig {
        normalization: NormalizationPolicy::default(),
        boilerplate: BoilerplatePolicy::default(),
        token_counter: Arc::new(WhitespaceTokenCounter),
    }
}

fn list_markdown_files(output_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut markdown_files = Vec::new();
    for entry in fs::read_dir(output_dir).map_err(|err| {
        format!(
            "failed to list markdown files in {}: {}",
            output_dir.display(),
            err
        )
    })? {
        let entry = entry
            .map_err(|err| format!("failed to read entry in {}: {}", output_dir.display(), err))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("md"))
            != Some(true)
        {
            continue;
        }
        markdown_files.push(path);
    }

    markdown_files.sort();
    Ok(markdown_files)
}

fn scan_article_path(
    index: usize,
    path: PathBuf,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
    config: &ContentPrepConfig,
) -> Result<ArticleScanOutcome, String> {
    let markdown = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let fields = match parse_frontmatter(&markdown) {
        Some(fields) => fields,
        None => {
            // Archive files (multi-doc format) start with "=====" and live in the
            // same output directory as article files — skip them silently at DEBUG.
            // Any other .md file without valid frontmatter is unexpected and warrants a WARN.
            if markdown.starts_with("=====") {
                engine_debug!(
                    "[briefing-loader] skipping {}: archive format (not an article)",
                    path.display()
                );
            } else {
                engine_warn!(
                    "[briefing-loader] skipping {}: no valid frontmatter",
                    path.display()
                );
            }
            return Ok(ArticleScanOutcome {
                index,
                package: None,
                missing_fetched_utc: false,
                malformed_fetched_utc: false,
            });
        }
    };
    let url = match fields
        .url
        .as_deref()
        .map(|u| u.trim())
        .filter(|u| !u.is_empty())
    {
        Some(url) => url.to_string(),
        None => {
            engine_warn!(
                "[briefing-loader] skipping {}: no url field",
                path.display()
            );
            return Ok(ArticleScanOutcome {
                index,
                package: None,
                missing_fetched_utc: false,
                malformed_fetched_utc: false,
            });
        }
    };

    let mut missing_fetched_utc = false;
    let mut malformed_fetched_utc = false;
    if let Some(since_dt) = since_utc {
        match &fields.fetched_utc {
            None => {
                missing_fetched_utc = true;
            }
            Some(raw) => match parse_rfc3339_utc("article", raw) {
                Err(_) => {
                    malformed_fetched_utc = true;
                }
                Ok(art_ts) => {
                    if art_ts < since_dt {
                        return Ok(ArticleScanOutcome {
                            index,
                            package: None,
                            missing_fetched_utc,
                            malformed_fetched_utc,
                        });
                    }
                }
            },
        }
    }

    let fetched_utc = fields.fetched_utc.clone();
    let clean_text = derive_clean_text(&markdown, &url, fields.title.as_deref(), config);
    Ok(ArticleScanOutcome {
        index,
        package: Some(ArticlePackage {
            url,
            source_title: fields.title,
            clean_text,
            fetched_utc,
        }),
        missing_fetched_utc,
        malformed_fetched_utc,
    })
}

/// Scan `output_dir` for markdown files, parse frontmatter, and derive clean text.
/// Packages are ordered by filename so callers can rely on deterministic order.
/// If `since_utc` is `Some`, articles with a `fetched_utc` older than the threshold are excluded.
/// Articles missing or with malformed `fetched_utc` are always included (with a summary warning).
fn scan_and_prepare_articles(
    output_dir: &Path,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Vec<ArticlePackage>, String> {
    scan_and_prepare_articles_with_progress(output_dir, since_utc, |_| {})
}

fn scan_and_prepare_articles_with_progress<F>(
    output_dir: &Path,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
    mut on_progress: F,
) -> Result<Vec<ArticlePackage>, String>
where
    F: FnMut(ArticleScanProgress),
{
    let config = build_content_prep_config();
    let markdown_files = list_markdown_files(output_dir)?;
    let files_total = markdown_files.len();

    let mut missing_fetched_utc_count: usize = 0;
    let mut malformed_fetched_utc_count: usize = 0;

    if files_total == 0 {
        return Ok(Vec::new());
    }

    let worker_count = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(files_total);
    let chunk_size = files_total.div_ceil(worker_count);
    let indexed_files: Vec<(usize, PathBuf)> = markdown_files.into_iter().enumerate().collect();
    let (tx, rx) = mpsc::channel();
    let mut outcomes = Vec::with_capacity(files_total);

    thread::scope(|scope| {
        for chunk in indexed_files.chunks(chunk_size) {
            let tx = tx.clone();
            let config = config.clone();
            let jobs = chunk.to_vec();
            scope.spawn(move || {
                for (index, path) in jobs {
                    let result = scan_article_path(index, path, since_utc, &config);
                    let _ = tx.send(result);
                }
            });
        }
        drop(tx);

        let mut first_error: Option<String> = None;
        for (index, result) in rx.into_iter().enumerate() {
            let scanned_count = index + 1;
            match result {
                Ok(outcome) => {
                    if outcome.missing_fetched_utc {
                        missing_fetched_utc_count += 1;
                    }
                    if outcome.malformed_fetched_utc {
                        malformed_fetched_utc_count += 1;
                    }
                    outcomes.push(outcome);
                }
                Err(err) => {
                    first_error.get_or_insert(err);
                }
            }
            on_progress(ArticleScanProgress {
                files_scanned: scanned_count,
                files_total,
            });
        }
        if let Some(err) = first_error {
            return Err(err);
        }
        Ok(())
    })?;

    outcomes.sort_by_key(|outcome| outcome.index);
    let packages: Vec<ArticlePackage> = outcomes
        .into_iter()
        .filter_map(|outcome| outcome.package)
        .collect();

    if missing_fetched_utc_count > 0 {
        engine_warn!(
            "[briefing-filter] {} article(s) missing fetched_utc — included",
            missing_fetched_utc_count
        );
    }
    if malformed_fetched_utc_count > 0 {
        engine_warn!(
            "[briefing-filter] {} article(s) had malformed fetched_utc — included",
            malformed_fetched_utc_count
        );
    }

    Ok(packages)
}

/// Budget shared by triage preparation and article summaries.
pub fn summary_preparation_budget(
    max_input_bytes: usize,
    registry: &PromptRegistry,
) -> Result<usize, String> {
    let template = registry
        .active_effective(PromptId::ArticleSummary)
        .ok_or_else(|| "summary prompt not registered".to_string())?;
    let overhead = crate::content_prep::compute_template_overhead(
        template.system_template(),
        template.user_template(),
        "content",
        &[],
    );
    max_input_bytes.checked_sub(overhead).ok_or_else(|| {
        format!("summary prompt overhead ({overhead}) exceeds max input budget ({max_input_bytes})")
    })
}

fn prepare_article(package: &ArticlePackage, budget: usize) -> LoadedArticle {
    let (prepared_text, _) = truncate_to_budget(package.clean_text.text(), budget);
    LoadedArticle {
        url: package.url.clone(),
        source_title: package.source_title.clone(),
        prepared_text,
        content_hash: package.clean_text.content_hash().to_string(),
        fetched_utc: package.fetched_utc.clone(),
    }
}

fn normalized_url_lookup_key(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
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
        return parsed.to_string().trim_end_matches('/').to_string();
    }

    trimmed.to_lowercase().trim_end_matches('/').to_string()
}

fn url_lookup_aliases(url: &str) -> Vec<String> {
    let key = normalized_url_lookup_key(url);
    if key.is_empty() {
        return vec![key];
    }

    let mut aliases = vec![key.clone()];
    if let Ok(parsed) = Url::parse(&key) {
        let push_scheme_variants = |aliases: &mut Vec<String>, base: &Url| match base.scheme() {
            "http" => {
                let mut https = base.clone();
                if https.set_scheme("https").is_ok() {
                    aliases.push(https.to_string().trim_end_matches('/').to_string());
                }
            }
            "https" => {
                let mut http = base.clone();
                if http.set_scheme("http").is_ok() {
                    aliases.push(http.to_string().trim_end_matches('/').to_string());
                }
            }
            _ => {}
        };

        if parsed.query().is_some() {
            let mut no_query = parsed.clone();
            no_query.set_query(None);
            aliases.push(no_query.to_string().trim_end_matches('/').to_string());
            push_scheme_variants(&mut aliases, &no_query);
        }

        push_scheme_variants(&mut aliases, &parsed);

        let Some(host) = parsed.host_str() else {
            aliases.sort();
            aliases.dedup();
            return aliases;
        };
        let lowered = host.to_lowercase();
        for prefix in ["www.", "eu.", "m.", "edition."] {
            if let Some(stripped) = lowered.strip_prefix(prefix) {
                let mut host_alias = parsed.clone();
                let alias_host = stripped.to_string();
                let _ = host_alias.set_host(Some(&alias_host));
                aliases.push(host_alias.to_string().trim_end_matches('/').to_string());
                push_scheme_variants(&mut aliases, &host_alias);
                if host_alias.query().is_some() {
                    let mut host_alias_no_query = host_alias.clone();
                    host_alias_no_query.set_query(None);
                    aliases.push(
                        host_alias_no_query
                            .to_string()
                            .trim_end_matches('/')
                            .to_string(),
                    );
                    push_scheme_variants(&mut aliases, &host_alias_no_query);
                }
            }
        }

        if parsed.host_str() == Some("newsroom.cisco.com")
            && parsed.path().starts_with("/content/r/")
        {
            let mut cisco_alias = parsed.clone();
            let new_path = parsed.path().replacen("/content/r/", "/c/r/", 1);
            cisco_alias.set_path(&new_path);
            aliases.push(cisco_alias.to_string().trim_end_matches('/').to_string());
            push_scheme_variants(&mut aliases, &cisco_alias);
            if cisco_alias.query().is_some() {
                let mut cisco_alias_no_query = cisco_alias.clone();
                cisco_alias_no_query.set_query(None);
                aliases.push(
                    cisco_alias_no_query
                        .to_string()
                        .trim_end_matches('/')
                        .to_string(),
                );
                push_scheme_variants(&mut aliases, &cisco_alias_no_query);
            }
        }
    }

    aliases.sort();
    aliases.dedup();
    aliases
}

/// Scan `output_dir` for markdown articles and return lightweight metadata for each.
/// Used by the batch replay benchmark to assemble its corpus metadata.
/// Articles with missing or malformed frontmatter are skipped (logged as warnings by the inner scanner).
pub fn scan_archive_article_metadata(output_dir: &Path) -> Result<Vec<ArchiveArticleMeta>, String> {
    let packages = scan_and_prepare_articles(output_dir, None)?;
    Ok(packages
        .into_iter()
        .map(|p| ArchiveArticleMeta {
            url: p.url,
            fetched_utc: p.fetched_utc,
            content_hash: Some(p.clean_text.content_hash().to_string()),
        })
        .collect())
}
