use crate::path_policy::is_confined_to;
use crate::rss_parse::{parse_feed_content, RssPollItem};
use crate::RssSeenSet;
use crate::SourceId;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePollResult {
    pub source_id: SourceId,
    pub urls: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SourcePollError {
    #[error("path {0} is outside allowed directories")]
    PathConfinementViolation(PathBuf),
    #[error("file not found: {0}")]
    FileNotFound(PathBuf),
    #[error("I/O error on {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("feed parse failed for {url}: {reason}")]
    FeedParseFailed { url: String, reason: String },
}

pub fn validate_source_file_path(
    config_dir: &Path,
    candidate: &Path,
    allowed_dirs: &[PathBuf],
) -> Result<PathBuf, SourcePollError> {
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        config_dir.join(candidate)
    };
    let canonical = match resolved.canonicalize() {
        Ok(path) => path,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(SourcePollError::FileNotFound(resolved))
        }
        Err(err) => {
            return Err(SourcePollError::Io {
                path: resolved,
                source: err,
            })
        }
    };

    if allowed_dirs
        .iter()
        .any(|allowed| is_confined_to(&canonical, allowed))
    {
        Ok(canonical)
    } else {
        Err(SourcePollError::PathConfinementViolation(canonical))
    }
}

pub fn poll_file_source(
    source_id: SourceId,
    path: &Path,
    config_dir: &Path,
    allowed_dirs: &[PathBuf],
    max_urls_per_poll: Option<usize>,
) -> Result<SourcePollResult, SourcePollError> {
    let canonical = validate_source_file_path(config_dir, path, allowed_dirs)?;
    let file = File::open(&canonical).map_err(|err| SourcePollError::Io {
        path: canonical.clone(),
        source: err,
    })?;
    let reader = BufReader::new(file);
    let limit = max_urls_per_poll.unwrap_or(usize::MAX);
    let mut urls = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|err| SourcePollError::Io {
            path: canonical.clone(),
            source: err,
        })?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        urls.push(trimmed.to_string());
        if urls.len() >= limit {
            break;
        }
    }
    Ok(SourcePollResult { source_id, urls })
}

pub fn poll_curated_source(
    source_id: SourceId,
    urls: &[String],
    max_urls_per_poll: Option<usize>,
) -> SourcePollResult {
    let limit = max_urls_per_poll.unwrap_or(urls.len());
    let selected = urls.iter().take(limit).cloned().collect::<Vec<_>>();
    SourcePollResult {
        source_id,
        urls: selected,
    }
}

#[allow(dead_code)]
pub fn poll_rss_source(
    source_id: SourceId,
    feed_bytes: &[u8],
    feed_url: &str,
    seen_set: &mut RssSeenSet,
    max_urls_per_poll: Option<usize>,
) -> Result<SourcePollResult, SourcePollError> {
    let entries = parse_feed_content(feed_bytes, feed_url).map_err(|err| {
        SourcePollError::FeedParseFailed {
            url: feed_url.to_string(),
            reason: err.to_string(),
        }
    })?;

    let unseen_entries = seen_set.filter_unseen_entries(source_id.as_str(), entries);
    let poll_items: Vec<RssPollItem> = unseen_entries
        .into_iter()
        .filter_map(|entry| entry.into_poll_item())
        .collect();

    let limit = max_urls_per_poll.unwrap_or(poll_items.len());
    let selected_urls = poll_items
        .into_iter()
        .take(limit)
        .map(|item| item.url)
        .collect();

    Ok(SourcePollResult {
        source_id,
        urls: selected_urls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn config_dir() -> TempDir {
        TempDir::new().expect("tempdir")
    }

    #[test]
    fn validate_relative_path_within_config_dir() {
        let temp = config_dir();
        let file = temp.path().join("urls.txt");
        fs::write(&file, "https://example.com\n").expect("write file");
        let allowed = [temp.path().to_path_buf()];
        let canonical = validate_source_file_path(temp.path(), Path::new("urls.txt"), &allowed)
            .expect("path ok");
        assert!(canonical.ends_with("urls.txt"));
    }

    #[test]
    fn validate_rejects_outside_dirs() {
        let temp = config_dir();
        let parent = temp.path().parent().unwrap().to_path_buf();
        let outside = parent.join("outside.txt");
        fs::write(&outside, "https://example.com\n").expect("write file");
        let err = validate_source_file_path(
            temp.path(),
            Path::new("../outside.txt"),
            &[temp.path().to_path_buf()],
        )
        .unwrap_err();
        assert!(matches!(err, SourcePollError::PathConfinementViolation(_)));
    }

    #[test]
    fn poll_file_source_respects_comments_and_limit() {
        let temp = config_dir();
        let file_path = temp.path().join("feeds.txt");
        fs::write(
            &file_path,
            "# comment\nhttps://one\n\nhttps://two\nhttps://three\n",
        )
        .expect("write file");

        let result = poll_file_source(
            SourceId::new("f").unwrap(),
            Path::new("feeds.txt"),
            temp.path(),
            &[temp.path().to_path_buf()],
            Some(2),
        )
        .expect("poll ok");
        assert_eq!(result.urls, vec!["https://one", "https://two"]);
    }

    #[test]
    fn poll_curated_source_limits_output() {
        let urls = vec![
            "https://a".to_string(),
            "https://b".to_string(),
            "https://c".to_string(),
        ];
        let result = poll_curated_source(SourceId::new("c").unwrap(), &urls, Some(2));
        assert_eq!(result.urls, vec!["https://a", "https://b"]);
    }

    fn rss_feed_content(items: &[(&str, Option<&str>)]) -> Vec<u8> {
        let items_xml: String = items
            .iter()
            .map(|(guid, link)| {
                let link_xml = match link {
                    Some(link) => format!(r#"<link>{}</link>"#, link),
                    None => "".to_string(),
                };
                format!(
                    r#"
                <item>
                    <title>{guid}</title>
                    <guid>{guid}</guid>
                    {link}
                </item>
            "#,
                    guid = guid,
                    link = link_xml
                )
            })
            .collect();

        format!(
            r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Test</title><link>https://example.com/</link><description>desc</description>{items}</channel></rss>"#,
            items = items_xml
        )
        .into_bytes()
    }

    #[test]
    fn poll_rss_source_returns_unseen_urls() {
        let mut seen = RssSeenSet::new();
        let feed = rss_feed_content(&[("g1", Some("https://example.com/1"))]);
        let result = poll_rss_source(
            SourceId::new("rss").unwrap(),
            &feed,
            "https://example.com/feed",
            &mut seen,
            None,
        )
        .expect("poll success");
        assert_eq!(result.urls, vec!["https://example.com/1"]);
    }

    #[test]
    fn poll_rss_source_filters_seen_items() {
        let mut seen = RssSeenSet::new();
        let feed = rss_feed_content(&[
            ("g1", Some("https://example.com/1")),
            ("g2", Some("https://example.com/2")),
        ]);
        let _ = poll_rss_source(
            SourceId::new("rss").unwrap(),
            &feed,
            "https://example.com/feed",
            &mut seen,
            None,
        );

        let second = poll_rss_source(
            SourceId::new("rss").unwrap(),
            &feed,
            "https://example.com/feed",
            &mut seen,
            None,
        )
        .expect("poll success");
        assert!(second.urls.is_empty());
    }

    #[test]
    fn poll_rss_source_applies_max_after_dedup() {
        let mut seen = RssSeenSet::new();
        let initial = rss_feed_content(&[
            ("g1", Some("https://example.com/1")),
            ("g2", Some("https://example.com/2")),
        ]);
        let _ = poll_rss_source(
            SourceId::new("rss").unwrap(),
            &initial,
            "https://example.com/feed",
            &mut seen,
            None,
        );

        let next = rss_feed_content(&[
            ("g1", Some("https://example.com/1")),
            ("g2", Some("https://example.com/2")),
            ("g3", Some("https://example.com/3")),
            ("g4", Some("https://example.com/4")),
        ]);
        let result = poll_rss_source(
            SourceId::new("rss").unwrap(),
            &next,
            "https://example.com/feed",
            &mut seen,
            Some(1),
        )
        .expect("poll success");
        assert_eq!(result.urls, vec!["https://example.com/3"]);
    }

    #[test]
    fn poll_rss_source_marks_no_url_entries_as_seen() {
        let mut seen = RssSeenSet::new();
        let feed = rss_feed_content(&[("nolink", None)]);
        let result = poll_rss_source(
            SourceId::new("rss").unwrap(),
            &feed,
            "https://example.com/feed",
            &mut seen,
            None,
        )
        .expect("poll success");
        assert!(result.urls.is_empty());

        let again = poll_rss_source(
            SourceId::new("rss").unwrap(),
            &feed,
            "https://example.com/feed",
            &mut seen,
            None,
        )
        .expect("poll success");
        assert!(again.urls.is_empty());
    }

    #[test]
    fn poll_rss_source_returns_parse_failure() {
        let mut seen = RssSeenSet::new();
        let err = poll_rss_source(
            SourceId::new("rss").unwrap(),
            b"not a feed",
            "https://example.com/feed",
            &mut seen,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, SourcePollError::FeedParseFailed { .. }));
    }
}
