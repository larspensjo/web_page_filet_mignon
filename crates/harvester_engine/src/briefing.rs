use std::{fs, path::Path, sync::Arc};

use engine_logging::{engine_info, engine_warn};

use crate::content_prep::{
    compute_prompt_overhead, derive_clean_text, truncate_to_budget, BoilerplatePolicy, CleanText,
    ContentBudget, ContentPrepConfig, NormalizationPolicy, PreparedCollection, PreparedInput,
};
use crate::frontmatter::parse_frontmatter;
use crate::llm::{PromptId, PromptRegistry};
use crate::token::WhitespaceTokenCounter;

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
fn scan_and_prepare_articles(output_dir: &Path) -> Result<Vec<ArticlePackage>, String> {
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

        let clean_text = derive_clean_text(&markdown, &url, fields.title.as_deref(), &config);
        packages.push(ArticlePackage {
            url,
            source_title: fields.title,
            clean_text,
        });
    }

    Ok(packages)
}

pub fn load_and_prepare_articles(
    output_dir: &Path,
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

    let packages = scan_and_prepare_articles(output_dir)?;

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

    let packages = scan_and_prepare_articles(output_dir)?;

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
