use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    sync::Arc,
};

use engine_logging::{engine_debug, engine_info, engine_warn};
use url::Url;

use crate::content_prep::{
    compute_prompt_overhead, derive_clean_text, truncate_to_budget, BoilerplatePolicy, CleanText,
    ContentBudget, ContentPrepConfig, NormalizationPolicy, PreparedCollection, PreparedInput,
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

const MIN_COLLECTION_PER_ARTICLE: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub matched_urls: usize,
}

struct ArticlePackage {
    url: String,
    source_title: Option<String>,
    clean_text: CleanText,
    /// RFC3339 UTC timestamp from frontmatter, preserved for downstream consumers.
    fetched_utc: Option<String>,
}

impl ArticlePackage {
    fn title(&self) -> &str {
        self.clean_text
            .report()
            .source_title
            .as_deref()
            .unwrap_or("untitled")
    }
}

fn article_header_length(idx: usize, article: &ArticlePackage) -> usize {
    format!("--- Article {}: {} ---", idx + 1, article.title()).len()
}

fn build_content_prep_config() -> ContentPrepConfig {
    ContentPrepConfig {
        normalization: NormalizationPolicy::default(),
        boilerplate: BoilerplatePolicy::default(),
        token_counter: Arc::new(WhitespaceTokenCounter),
    }
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

    let files_total = markdown_files.len();

    let mut missing_fetched_utc_count: usize = 0;
    let mut malformed_fetched_utc_count: usize = 0;

    let mut packages = Vec::with_capacity(markdown_files.len());
    for (index, path) in markdown_files.into_iter().enumerate() {
        let files_scanned = index + 1;
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
                on_progress(ArticleScanProgress {
                    files_scanned,
                    files_total,
                    matched_urls: packages.len(),
                });
                continue;
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
                on_progress(ArticleScanProgress {
                    files_scanned,
                    files_total,
                    matched_urls: packages.len(),
                });
                continue;
            }
        };

        if let Some(since_dt) = since_utc {
            match &fields.fetched_utc {
                None => {
                    missing_fetched_utc_count += 1;
                }
                Some(raw) => match parse_rfc3339_utc("article", raw) {
                    Err(_) => {
                        malformed_fetched_utc_count += 1;
                    }
                    Ok(art_ts) => {
                        if art_ts < since_dt {
                            on_progress(ArticleScanProgress {
                                files_scanned,
                                files_total,
                                matched_urls: packages.len(),
                            });
                            continue;
                        }
                    }
                },
            }
        }

        let fetched_utc = fields.fetched_utc.clone();
        let clean_text = derive_clean_text(&markdown, &url, fields.title.as_deref(), &config);
        packages.push(ArticlePackage {
            url,
            source_title: fields.title,
            clean_text,
            fetched_utc,
        });
        on_progress(ArticleScanProgress {
            files_scanned,
            files_total,
            matched_urls: packages.len(),
        });
    }

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

pub fn load_and_prepare_articles(
    output_dir: &Path,
    max_input_bytes: usize,
    registry: &PromptRegistry,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(Vec<LoadedArticle>, String), String> {
    let packages = scan_and_prepare_articles(output_dir, since_utc)?;
    prepare_loaded_articles_and_collection(packages, max_input_bytes, registry)
}

fn prepare_loaded_articles_and_collection(
    packages: Vec<ArticlePackage>,
    max_input_bytes: usize,
    registry: &PromptRegistry,
) -> Result<(Vec<LoadedArticle>, String), String> {
    let summary_template = registry
        .active(PromptId::ArticleSummary)
        .ok_or_else(|| "summary prompt not registered".to_string())?;
    let briefing_template = registry
        .active(PromptId::AggregateBriefing)
        .ok_or_else(|| "aggregate briefing prompt not registered".to_string())?;

    let summary_overhead = compute_prompt_overhead(summary_template, "content", &[]);
    let summary_budget = max_input_bytes
        .checked_sub(summary_overhead)
        .ok_or_else(|| {
            format!(
                "summary prompt overhead ({}) exceeds max input budget ({})",
                summary_overhead, max_input_bytes
            )
        })?;

    let briefing_overhead = compute_prompt_overhead(briefing_template, "collection", &[]);
    let collection_budget = max_input_bytes
        .checked_sub(briefing_overhead)
        .ok_or_else(|| {
            format!(
                "briefing prompt overhead ({}) exceeds max input budget ({})",
                briefing_overhead, max_input_bytes
            )
        })?;

    if packages.is_empty() {
        return Ok((Vec::new(), String::new()));
    }

    let mut loaded_articles = Vec::with_capacity(packages.len());
    for package in packages.iter() {
        let (bounded_text, _) = truncate_to_budget(package.clean_text.text(), summary_budget);
        loaded_articles.push(LoadedArticle {
            url: package.url.clone(),
            source_title: package.source_title.clone(),
            prepared_text: bounded_text,
            content_hash: package.clean_text.content_hash().to_string(),
            fetched_utc: package.fetched_utc.clone(),
        });
    }

    let total_articles = packages.len();
    let max_header_len = packages
        .iter()
        .enumerate()
        .map(|(idx, package)| article_header_length(idx, package))
        .max()
        .unwrap_or(0);
    let separator_overhead = max_header_len + 3;

    let budget = ContentBudget::new(collection_budget);
    let mut selected_count = total_articles;
    let allocation = loop {
        if selected_count == 0 {
            break None;
        }
        if let Some(allocation) = budget.allocate_equal(
            selected_count,
            separator_overhead,
            MIN_COLLECTION_PER_ARTICLE,
        ) {
            break Some((selected_count, allocation));
        }
        selected_count -= 1;
    };

    let (selected_count, allocations) = match allocation {
        Some(result) => result,
        None => {
            return Err("collection budget too small to include any article".to_string());
        }
    };

    if selected_count < total_articles {
        engine_info!(
            "[briefing-loader] limiting collection to {} of {} articles due to budget",
            selected_count,
            total_articles
        );
    }

    let selected_packages: Vec<_> = packages.into_iter().take(selected_count).collect();
    let prepared_inputs: Vec<_> = selected_packages
        .into_iter()
        .zip(allocations)
        .map(|(package, budget)| PreparedInput::from_clean_text(package.clean_text, budget))
        .collect();
    let collection_text = PreparedCollection::from_inputs(prepared_inputs)
        .text()
        .to_string();

    Ok((loaded_articles, collection_text))
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

pub fn load_and_prepare_articles_filtered(
    output_dir: &Path,
    max_input_bytes: usize,
    registry: &PromptRegistry,
    ordered_urls: &[String],
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(Vec<LoadedArticle>, String), String> {
    load_and_prepare_articles_filtered_with_progress(
        output_dir,
        max_input_bytes,
        registry,
        ordered_urls,
        since_utc,
        |_| {},
    )
}

pub fn load_and_prepare_articles_filtered_with_progress<F>(
    output_dir: &Path,
    max_input_bytes: usize,
    registry: &PromptRegistry,
    ordered_urls: &[String],
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
    mut on_progress: F,
) -> Result<(Vec<LoadedArticle>, String), String>
where
    F: FnMut(ArticleScanProgress),
{
    if ordered_urls.is_empty() {
        return Ok((Vec::new(), String::new()));
    }

    if ordered_urls.len() == 1 {
        let selected_url = &ordered_urls[0];
        if let Some(package) = find_article_package_for_selected_url_with_progress(
            output_dir,
            selected_url,
            since_utc,
            &mut on_progress,
        )? {
            return prepare_loaded_articles_and_collection(
                vec![package],
                max_input_bytes,
                registry,
            );
        }
        // When a since_utc checkpoint is active, old articles are intentionally excluded
        // from the corpus — a missing URL is expected, not a data-integrity problem.
        if since_utc.is_some() {
            engine_debug!(
                "[briefing-loader] selected url not in corpus (outside checkpoint window): {}",
                selected_url
            );
        } else {
            engine_warn!(
                "[briefing-loader] selected url missing from corpus: {}",
                selected_url
            );
        }
        return Ok((Vec::new(), String::new()));
    }

    let packages =
        scan_and_prepare_articles_with_progress(output_dir, since_utc, &mut on_progress)?;
    let mut indexed: HashMap<String, ArticlePackage> = HashMap::with_capacity(packages.len());
    for package in packages {
        for key in url_lookup_aliases(&package.url) {
            indexed.entry(key).or_insert_with(|| ArticlePackage {
                url: package.url.clone(),
                source_title: package.source_title.clone(),
                clean_text: package.clean_text.clone(),
                fetched_utc: package.fetched_utc.clone(),
            });
        }
    }

    let mut selected_packages = Vec::with_capacity(ordered_urls.len());
    for url in ordered_urls {
        let matched = url_lookup_aliases(url)
            .into_iter()
            .find_map(|key| indexed.get(&key));
        match matched {
            Some(package) => selected_packages.push(ArticlePackage {
                url: package.url.clone(),
                source_title: package.source_title.clone(),
                clean_text: package.clean_text.clone(),
                fetched_utc: package.fetched_utc.clone(),
            }),
            None => {
                // When a since_utc checkpoint is active, old articles are intentionally excluded
                // from the corpus — a missing URL is expected, not a data-integrity problem.
                if since_utc.is_some() {
                    engine_debug!(
                        "[briefing-loader] selected url not in corpus (outside checkpoint window): {}",
                        url
                    );
                } else {
                    engine_warn!(
                        "[briefing-loader] selected url missing from corpus: {}",
                        url
                    );
                }
            }
        }
    }

    prepare_loaded_articles_and_collection(selected_packages, max_input_bytes, registry)
}

fn find_article_package_for_selected_url_with_progress<F>(
    output_dir: &Path,
    selected_url: &str,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
    mut on_progress: F,
) -> Result<Option<ArticlePackage>, String>
where
    F: FnMut(ArticleScanProgress),
{
    let config = build_content_prep_config();
    let selected_aliases: HashSet<String> = url_lookup_aliases(selected_url).into_iter().collect();
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

    let files_total = markdown_files.len();

    for (index, path) in markdown_files.into_iter().enumerate() {
        let files_scanned = index + 1;
        let markdown = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
        let Some(fields) = parse_frontmatter(&markdown) else {
            on_progress(ArticleScanProgress {
                files_scanned,
                files_total,
                matched_urls: 0,
            });
            continue;
        };
        let Some(url) = fields
            .url
            .as_deref()
            .map(|u| u.trim())
            .filter(|u| !u.is_empty())
            .map(ToString::to_string)
        else {
            on_progress(ArticleScanProgress {
                files_scanned,
                files_total,
                matched_urls: 0,
            });
            continue;
        };

        if let Some(since_dt) = since_utc {
            if let Some(raw_fetched_utc) = &fields.fetched_utc {
                if let Ok(article_ts) = parse_rfc3339_utc("article", raw_fetched_utc) {
                    if article_ts < since_dt {
                        on_progress(ArticleScanProgress {
                            files_scanned,
                            files_total,
                            matched_urls: 0,
                        });
                        continue;
                    }
                }
            }
        }

        let article_aliases = url_lookup_aliases(&url);
        if !article_aliases
            .iter()
            .any(|alias| selected_aliases.contains(alias))
        {
            on_progress(ArticleScanProgress {
                files_scanned,
                files_total,
                matched_urls: 0,
            });
            continue;
        }

        let clean_text = derive_clean_text(&markdown, &url, fields.title.as_deref(), &config);
        on_progress(ArticleScanProgress {
            files_scanned,
            files_total,
            matched_urls: 1,
        });
        return Ok(Some(ArticlePackage {
            url,
            source_title: fields.title,
            clean_text,
            fetched_utc: fields.fetched_utc,
        }));
    }

    Ok(None)
}

pub fn load_and_prepare_articles_for_triage(
    output_dir: &Path,
    max_input_bytes: usize,
    registry: &PromptRegistry,
) -> Result<Vec<LoadedArticle>, String> {
    let triage_template = registry
        .active(PromptId::ArticleTriage)
        .ok_or_else(|| "triage prompt not registered".to_string())?;
    let triage_overhead = compute_prompt_overhead(triage_template, "content", &[]);
    let triage_budget = max_input_bytes
        .checked_sub(triage_overhead)
        .ok_or_else(|| {
            format!(
                "triage prompt overhead ({}) exceeds max input budget ({})",
                triage_overhead, max_input_bytes
            )
        })?;

    let packages = scan_and_prepare_articles(output_dir, None)?;

    if packages.is_empty() {
        return Ok(Vec::new());
    }

    let mut loaded_articles = Vec::with_capacity(packages.len());
    for package in packages.iter() {
        let (bounded_text, _) = truncate_to_budget(package.clean_text.text(), triage_budget);
        loaded_articles.push(LoadedArticle {
            url: package.url.clone(),
            source_title: package.source_title.clone(),
            prepared_text: bounded_text,
            content_hash: package.clean_text.content_hash().to_string(),
            fetched_utc: package.fetched_utc.clone(),
        });
    }

    Ok(loaded_articles)
}

/// Load articles from exact archive file paths (for imported-corpus post-actions).
///
/// Unlike `load_and_prepare_articles_filtered`, this function resolves articles by
/// persisted path rather than URL-based directory scanning, so duplicate canonical URLs
/// within the same imported batch are all included.
pub fn load_and_prepare_articles_by_path(
    paths: &[std::path::PathBuf],
    max_input_bytes: usize,
    registry: &PromptRegistry,
) -> Result<(Vec<LoadedArticle>, String), String> {
    if paths.is_empty() {
        return Ok((Vec::new(), String::new()));
    }

    let config = build_content_prep_config();
    let mut packages = Vec::with_capacity(paths.len());

    for path in paths {
        let markdown = fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
        let fields = match parse_frontmatter(&markdown) {
            Some(fields) => fields,
            None => {
                engine_warn!(
                    "[briefing-loader] skipping {}: no valid frontmatter",
                    path.display()
                );
                continue;
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
                continue;
            }
        };
        let fetched_utc = fields.fetched_utc.clone();
        let clean_text = derive_clean_text(&markdown, &url, fields.title.as_deref(), &config);
        packages.push(ArticlePackage {
            url,
            source_title: fields.title,
            clean_text,
            fetched_utc,
        });
    }

    prepare_loaded_articles_and_collection(packages, max_input_bytes, registry)
}

/// Scan `output_dir` for markdown articles and return lightweight metadata for each.
/// Used by the entity index rebuild procedure to join against the triage/summary caches.
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
