use std::{collections::HashMap, fs, path::Path, sync::Arc};

use engine_logging::{engine_info, engine_warn};
use url::Url;

use crate::content_prep::{
    compute_prompt_overhead, derive_clean_text, truncate_to_budget, BoilerplatePolicy, CleanText,
    ContentBudget, ContentPrepConfig, NormalizationPolicy, PreparedCollection, PreparedInput,
};
use crate::frontmatter::parse_frontmatter;
use crate::llm::{PromptId, PromptRegistry};
use crate::token::WhitespaceTokenCounter;

fn parse_rfc3339_utc(label: &str, value: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    let snippet = if value.len() > 50 { &value[..50] } else { value };
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
}

struct ArticlePackage {
    url: String,
    source_title: Option<String>,
    clean_text: CleanText,
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

    let mut missing_fetched_utc_count: usize = 0;
    let mut malformed_fetched_utc_count: usize = 0;

    let mut packages = Vec::with_capacity(markdown_files.len());
    for path in markdown_files {
        let markdown = fs::read_to_string(&path)
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
                            continue;
                        }
                    }
                },
            }
        }

        let clean_text = derive_clean_text(&markdown, &url, fields.title.as_deref(), &config);
        packages.push(ArticlePackage {
            url,
            source_title: fields.title,
            clean_text,
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
    if let Ok(mut parsed) = Url::parse(&key) {
        let Some(host) = parsed.host_str() else {
            return aliases;
        };
        let lowered = host.to_lowercase();
        let host_without_prefix = if let Some(stripped) = lowered.strip_prefix("www.") {
            Some(stripped.to_string())
        } else {
            lowered
                .strip_prefix("eu.")
                .map(|stripped| stripped.to_string())
        };

        if let Some(alias_host) = host_without_prefix {
            let _ = parsed.set_host(Some(&alias_host));
            aliases.push(parsed.to_string().trim_end_matches('/').to_string());
        }
    }

    aliases
}

pub fn load_and_prepare_articles_filtered(
    output_dir: &Path,
    max_input_bytes: usize,
    registry: &PromptRegistry,
    ordered_urls: &[String],
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(Vec<LoadedArticle>, String), String> {
    if ordered_urls.is_empty() {
        return Ok((Vec::new(), String::new()));
    }

    let packages = scan_and_prepare_articles(output_dir, since_utc)?;
    let mut indexed: HashMap<String, ArticlePackage> = HashMap::with_capacity(packages.len());
    for package in packages {
        for key in url_lookup_aliases(&package.url) {
            indexed.entry(key).or_insert_with(|| ArticlePackage {
                url: package.url.clone(),
                source_title: package.source_title.clone(),
                clean_text: package.clean_text.clone(),
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
            }),
            None => {
                engine_warn!(
                    "[briefing-loader] selected url missing from corpus: {}",
                    url
                );
            }
        }
    }

    prepare_loaded_articles_and_collection(selected_packages, max_input_bytes, registry)
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
        });
    }

    Ok(loaded_articles)
}
