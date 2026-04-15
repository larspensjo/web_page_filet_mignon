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
        let overall_start = std::time::Instant::now();
        let mut articles = Vec::new();
        let mut discovered_md_files = 0usize;
        let mut accepted_articles = 0usize;
        let mut total_bytes = 0usize;
        let mut read_ms = 0u128;
        let mut parse_ms = 0u128;
        let mut slow_files: Vec<(u128, String)> = Vec::new();

        let read_dir_start = std::time::Instant::now();
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
        let read_dir_ms = read_dir_start.elapsed().as_millis();

        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            discovered_md_files += 1;
            let filename = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let file_start = std::time::Instant::now();
            let read_start = std::time::Instant::now();
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let read_elapsed = read_start.elapsed().as_millis();
            read_ms += read_elapsed;
            total_bytes += content.len();
            let parse_start = std::time::Instant::now();
            let fields = match parse_frontmatter(&content) {
                Some(f) => f,
                None => continue,
            };
            parse_ms += parse_start.elapsed().as_millis();
            accepted_articles += 1;
            articles.push(ArticleEntry {
                filename,
                path,
                title: fields.title,
                url: fields.url,
                fetched_utc: fields.fetched_utc,
                token_count: fields.token_count,
                content,
            });
            let file_elapsed = file_start.elapsed().as_millis();
            if slow_files.len() < 5 {
                slow_files.push((file_elapsed, articles.last().unwrap().filename.clone()));
            } else if let Some((min_index, min_value)) = slow_files
                .iter()
                .enumerate()
                .min_by_key(|(_, (elapsed, _))| *elapsed)
                .map(|(index, (elapsed, _))| (index, *elapsed))
            {
                if file_elapsed > min_value {
                    slow_files[min_index] =
                        (file_elapsed, articles.last().unwrap().filename.clone());
                }
            }
        }

        slow_files.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        let slow_file_summary = slow_files
            .into_iter()
            .map(|(elapsed, filename)| format!("{filename}:{elapsed}ms"))
            .collect::<Vec<_>>()
            .join(", ");
        engine_logging::engine_info!(
            "article_index profile: read_dir_ms={} discovered_md_files={} accepted_articles={} total_bytes={} read_ms={} parse_ms={} total_ms={} slowest_files=[{}]",
            read_dir_ms,
            discovered_md_files,
            accepted_articles,
            total_bytes,
            read_ms,
            parse_ms,
            overall_start.elapsed().as_millis(),
            slow_file_summary
        );

        Self { articles }
    }
}
