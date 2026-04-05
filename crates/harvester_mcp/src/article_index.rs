use std::path::{Path, PathBuf};

use harvester_engine::parse_frontmatter;

pub struct ArticleEntry {
    pub filename: String,
    #[allow(dead_code)]
    pub path: PathBuf,
    pub title: Option<String>,
    pub url: Option<String>,
    pub fetched_utc: Option<String>,
    pub token_count: Option<u32>,
    pub content: String,
}

pub struct ArticleIndex {
    pub articles: Vec<ArticleEntry>,
}

impl ArticleIndex {
    /// Load all eligible articles from the output dir.
    /// "Eligible" = valid Harvester frontmatter (parse_frontmatter returns Some).
    /// Non-article files (no frontmatter) are skipped silently.
    pub fn load(output_dir: &Path) -> Self {
        let mut articles = Vec::new();

        let read_dir = match std::fs::read_dir(output_dir) {
            Ok(rd) => rd,
            Err(e) => {
                engine_logging::engine_info!(
                    "article_index: could not read output_dir {:?}: {}",
                    output_dir,
                    e
                );
                return Self { articles };
            }
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let filename = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let fields = match parse_frontmatter(&content) {
                Some(f) => f,
                None => continue,
            };
            articles.push(ArticleEntry {
                filename,
                path,
                title: fields.title,
                url: fields.url,
                fetched_utc: fields.fetched_utc,
                token_count: fields.token_count,
                content,
            });
        }

        Self { articles }
    }
}
