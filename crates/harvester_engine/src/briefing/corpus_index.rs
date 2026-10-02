use super::*;
use std::time::SystemTime;

/// Preparation validity is separate from the URL/content identity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HeldArticle {
    pub url: String,
    pub content_hash: String,
    pub preparation_budget: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WindowArticle {
    pub url: String,
    pub content_hash: String,
    pub source_title: Option<String>,
    pub fetched_utc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TriageArticleDelta {
    pub members: Vec<WindowArticle>,
    pub preparation_budget: usize,
    pub articles: Vec<LoadedArticle>,
}

impl TriageArticleDelta {
    /// Full-window adapter for fixtures and callers without held preparation.
    pub fn full_window(articles: Vec<LoadedArticle>, preparation_budget: usize) -> Self {
        let members = articles
            .iter()
            .map(|a| WindowArticle {
                url: a.url.clone(),
                content_hash: a.content_hash.clone(),
                source_title: a.source_title.clone(),
                fetched_utc: a.fetched_utc.clone(),
            })
            .collect();
        Self {
            members,
            preparation_budget,
            articles,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CorpusScanStats {
    pub files: usize,
    pub reused: usize,
    pub read: usize,
    pub reprepared: usize,
    pub removed: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct Fingerprint {
    length: u64,
    modified: SystemTime,
}

#[derive(Debug)]
struct IndexedFile {
    fingerprint: Fingerprint,
    // None records a non-article, including archive artifacts.
    article: Option<WindowArticle>,
}

/// Process-lifetime metadata only. Never serialized and never retains article text.
#[derive(Debug, Default)]
pub struct CorpusScanIndex {
    files: HashMap<PathBuf, IndexedFile>,
}

impl CorpusScanIndex {
    /// All valid frontmatter fetch times, without a display cutoff or text preparation.
    pub fn fetch_times(&mut self, output: &Path) -> Result<HashMap<String, String>, String> {
        use std::io::{BufRead, BufReader};
        // Recovery needs only the leading metadata block. Do not load, hash or
        // prepare article bodies, or build a second process-lifetime corpus index.
        let mut paths = list_markdown_files(output)?;
        let linked = output.join("linked");
        if linked.is_dir() {
            paths.extend(list_markdown_files(&linked)?);
        }
        let mut times = HashMap::new();
        for path in paths {
            let file = fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let mut header = String::new();
            for (index, line) in BufReader::new(file).lines().enumerate() {
                let line = line.map_err(|e| format!("{}: {e}", path.display()))?;
                if index == 0 && line.trim() != "---" {
                    break;
                }
                header.push_str(&line);
                header.push('\n');
                if index > 0 && line.trim() == "---" {
                    break;
                }
                // A missing delimiter must not turn recovery into a body scan.
                if header.len() > 64 * 1024 {
                    break;
                }
            }
            if let Some(fields) = parse_frontmatter(&header) {
                if let (Some(url), Some(date)) = (fields.url, fields.fetched_utc) {
                    if chrono::DateTime::parse_from_rfc3339(&date).is_ok() {
                        times.entry(crate::archive_url_key(&url)).or_insert(date);
                    }
                }
            }
        }
        Ok(times)
    }

    pub fn clear(&mut self) {
        self.files.clear();
    }

    /// Lists every scan, reuses unchanged metadata, and prepares only missing or
    /// differently budgeted identities. Membership follows requested download order.
    #[allow(clippy::too_many_arguments)]
    pub fn load_delta(
        &mut self,
        output_dir: &Path,
        max_input_bytes: usize,
        registry: &PromptRegistry,
        ordered_urls: &[String],
        since: Option<chrono::DateTime<chrono::Utc>>,
        held: &[HeldArticle],
        mut on_progress: impl FnMut(ArticleScanProgress),
    ) -> Result<(TriageArticleDelta, CorpusScanStats), String> {
        let budget = summary_preparation_budget(max_input_bytes, registry)?;
        let paths = list_markdown_files(output_dir)?;
        let mut stats = CorpusScanStats {
            files: paths.len(),
            ..Default::default()
        };
        let present: HashSet<_> = paths.iter().cloned().collect();
        let before = self.files.len();
        self.files.retain(|path, _| present.contains(path));
        stats.removed = before - self.files.len();
        let held: HashMap<_, _> = held
            .iter()
            .map(|h| {
                (
                    (h.url.as_str(), h.content_hash.as_str()),
                    h.preparation_budget,
                )
            })
            .collect();
        let config = build_content_prep_config();
        let mut changed = Vec::new();
        let mut fingerprints = HashMap::new();
        for (i, path) in paths.iter().enumerate() {
            let metadata = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let fingerprint = Fingerprint {
                length: metadata.len(),
                modified: metadata
                    .modified()
                    .map_err(|e| format!("{}: {e}", path.display()))?,
            };
            if self
                .files
                .get(path)
                .is_some_and(|f| f.fingerprint == fingerprint)
            {
                stats.reused += 1;
            } else {
                changed.push((i, path.clone()));
                fingerprints.insert(path.clone(), fingerprint);
            }
            on_progress(ArticleScanProgress {
                files_scanned: i + 1,
                files_total: stats.files,
            });
        }
        stats.read = changed.len();
        let mut fresh = HashMap::new();
        for outcome in scan_paths(&changed, &config)? {
            let path = &paths[outcome.index];
            let article = outcome.package.as_ref().map(|p| WindowArticle {
                url: p.url.clone(),
                content_hash: p.clean_text.content_hash().to_string(),
                source_title: p.source_title.clone(),
                fetched_utc: p.fetched_utc.clone(),
            });
            if let Some(package) = outcome.package {
                fresh.insert(path.clone(), package);
            }
            self.files.insert(
                path.clone(),
                IndexedFile {
                    fingerprint: fingerprints.remove(path).expect("changed fingerprint"),
                    article,
                },
            );
        }
        let mut indexed_aliases = HashMap::new();
        let mut missing_dates = 0;
        let mut malformed_dates = 0;
        for (i, path) in paths.iter().enumerate() {
            if let Some(member) = self.files[path].article.as_ref() {
                let inside_window = match (since, member.fetched_utc.as_deref()) {
                    (Some(_), None) => {
                        missing_dates += 1;
                        true
                    }
                    (Some(since), Some(raw)) => match parse_rfc3339_utc("article", raw) {
                        Ok(date) => date >= since,
                        Err(_) => {
                            malformed_dates += 1;
                            true
                        }
                    },
                    _ => true,
                };
                if inside_window {
                    for alias in url_lookup_aliases(&member.url) {
                        indexed_aliases
                            .entry(alias)
                            .or_insert_with(|| (i, path.clone(), member.clone()));
                    }
                }
            }
        }
        if missing_dates > 0 {
            engine_warn!(
                "[briefing-filter] {} article(s) missing fetched_utc — included",
                missing_dates
            );
        }
        if malformed_dates > 0 {
            engine_warn!(
                "[briefing-filter] {} article(s) had malformed fetched_utc — included",
                malformed_dates
            );
        }
        let mut selected = Vec::new();
        let mut selected_urls = HashSet::new();
        for (request, url) in ordered_urls.iter().enumerate() {
            let matched = url_lookup_aliases(url)
                .into_iter()
                .find_map(|alias| indexed_aliases.get(&alias));
            if let Some((i, path, member)) = matched {
                if selected_urls.insert(member.url.clone()) {
                    selected.push((request, *i, path.clone(), member.clone()));
                }
            }
        }
        let mut reprepare = Vec::new();
        for (_, i, path, member) in &selected {
            if held.get(&(member.url.as_str(), member.content_hash.as_str())) != Some(&budget)
                && !fresh.contains_key(path)
            {
                reprepare.push((*i, path.clone()));
            }
        }
        stats.reprepared = reprepare.len();
        for outcome in scan_paths(&reprepare, &config)? {
            let path = &paths[outcome.index];
            fresh.insert(
                path.clone(),
                outcome.package.ok_or_else(|| {
                    format!("article changed during preparation: {}", path.display())
                })?,
            );
        }
        selected.sort_by_key(|(request, _, _, _)| *request);
        let mut delta = TriageArticleDelta {
            members: Vec::with_capacity(selected.len()),
            preparation_budget: budget,
            articles: Vec::new(),
        };
        for (_, _, path, member) in selected {
            if held.get(&(member.url.as_str(), member.content_hash.as_str())) != Some(&budget) {
                let package = fresh.remove(&path).ok_or_else(|| {
                    format!("article changed during preparation: {}", path.display())
                })?;
                if package.clean_text.content_hash() != member.content_hash
                    || package.url != member.url
                {
                    return Err(format!(
                        "article changed during preparation: {}",
                        path.display()
                    ));
                }
                delta.articles.push(prepare_article(&package, budget));
            }
            delta.members.push(member);
        }
        Ok((delta, stats))
    }
}

/// The existing full loader uses the same core-count chunking for file reads.
fn scan_paths(
    jobs: &[(usize, PathBuf)],
    config: &ContentPrepConfig,
) -> Result<Vec<ArticleScanOutcome>, String> {
    if jobs.is_empty() {
        return Ok(Vec::new());
    }
    let workers = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(jobs.len());
    let chunk_size = jobs.len().div_ceil(workers);
    let (tx, rx) = mpsc::channel();
    thread::scope(|scope| {
        for chunk in jobs.chunks(chunk_size) {
            let tx = tx.clone();
            let config = config.clone();
            let chunk = chunk.to_vec();
            scope.spawn(move || {
                for (index, path) in chunk {
                    let _ = tx.send(scan_article_path(index, path, None, &config));
                }
            });
        }
        drop(tx);
        rx.into_iter().collect::<Result<Vec<_>, _>>()
    })
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    #[test]
    fn fetch_time_recovery_reads_only_frontmatter_in_root_and_legacy_linked_articles() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("linked")).unwrap();
        for relative in ["root.md", "linked/old.md"] {
            let mut bytes = format!("---\nurl: https://example.com/{relative}\nfetched_utc: 2026-09-20T09:15:00Z\n---\n").into_bytes();
            // Invalid UTF-8 body proves recovery never decodes/prepares article text.
            bytes.extend_from_slice(&[0xff, 0xfe]);
            fs::write(dir.path().join(relative), bytes).unwrap();
        }
        let mut index = CorpusScanIndex::default();
        let times = index.fetch_times(dir.path()).unwrap();
        assert_eq!(times.len(), 2);
        assert_eq!(times["https://example.com/root.md"], "2026-09-20T09:15:00Z");
        assert!(index.files.is_empty());
    }
}
