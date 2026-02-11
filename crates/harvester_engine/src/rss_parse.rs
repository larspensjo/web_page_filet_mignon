use chrono::{DateTime, Utc};
use feed_rs::model::{Entry, Link};
use thiserror::Error;
use url::Url;

/// Parsed feed item metadata, before filtering for usable URLs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeedEntry {
    pub guid: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub published: Option<DateTime<Utc>>,
}

/// Poll-ready feed item with a guaranteed URL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RssPollItem {
    pub guid: String,
    pub url: String,
    pub title: Option<String>,
    pub published: Option<DateTime<Utc>>,
}

impl FeedEntry {
    /// Returns a `RssPollItem` if a usable URL was extracted.
    pub fn into_poll_item(self) -> Option<RssPollItem> {
        self.url.map(|url| RssPollItem {
            guid: self.guid,
            url,
            title: self.title,
            published: self.published,
        })
    }
}

/// Error returned when RSS/Atom/JSON feed parsing fails.
#[derive(Debug, Error)]
pub enum FeedParseError {
    #[error("feed parse failed: {reason}")]
    ParseFailed { reason: String },
}

/// Parses raw feed bytes into `FeedEntry` metadata records.
pub fn parse_feed_content(raw: &[u8], feed_url: &str) -> Result<Vec<FeedEntry>, FeedParseError> {
    let feed = feed_rs::parser::parse(raw).map_err(|err| FeedParseError::ParseFailed {
        reason: err.to_string(),
    })?;

    let base_url = Url::parse(feed_url).map_err(|err| FeedParseError::ParseFailed {
        reason: format!("invalid feed URL {}: {}", feed_url, err),
    })?;

    let entries = feed
        .entries
        .into_iter()
        .filter_map(|entry| build_feed_entry(entry, &base_url))
        .collect();

    Ok(entries)
}

fn build_feed_entry(entry: Entry, feed_base: &Url) -> Option<FeedEntry> {
    let guid_trimmed = entry.id.trim();
    if guid_trimmed.is_empty() {
        return None;
    }

    let entry_base_url = entry.base.as_deref().and_then(|base| Url::parse(base).ok());

    let url = select_entry_url(&entry, entry_base_url.as_ref(), feed_base)
        .or_else(|| parse_guid_as_url(guid_trimmed));

    Some(FeedEntry {
        guid: guid_trimmed.to_string(),
        url,
        title: entry.title.map(|text| text.content),
        published: entry.published,
    })
}

fn select_entry_url(entry: &Entry, entry_base: Option<&Url>, feed_base: &Url) -> Option<String> {
    let mut fallback = None;

    for link in &entry.links {
        if let Some(resolved) = resolve_link_href(link, entry_base, feed_base) {
            if is_alternate_rel(link.rel.as_deref()) {
                return Some(resolved);
            }
            if fallback.is_none() {
                fallback = Some(resolved);
            }
        }
    }

    fallback
}

fn resolve_link_href(link: &Link, entry_base: Option<&Url>, feed_base: &Url) -> Option<String> {
    let href = link.href.trim();
    if href.is_empty() {
        return None;
    }

    if let Ok(url) = Url::parse(href) {
        return Some(url.into());
    }

    if let Some(base) = entry_base {
        if let Ok(url) = base.join(href) {
            return Some(url.into());
        }
    }

    feed_base.join(href).ok().map(|url| url.into())
}

fn parse_guid_as_url(guid: &str) -> Option<String> {
    Url::parse(guid).ok().map(|url| url.into())
}

fn is_alternate_rel(rel: Option<&str>) -> bool {
    matches!(rel, Some(value) if value.eq_ignore_ascii_case("alternate"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn rss_feed_fixture() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <rss version="2.0">
            <channel>
                <title>Example &amp; Friends</title>
                <link>https://example.com/</link>
                <description>Test feed</description>
                <item>
                    <title>Relative Link</title>
                    <link>./relative-post</link>
                    <guid>item-relative</guid>
                    <pubDate>Sun, 06 Sep 2009 16:20:00 +0000</pubDate>
                </item>
                <item>
                    <title>Guid as URL</title>
                    <guid>https://example.com/guid-link</guid>
                </item>
            </channel>
        </rss>"#
    }

    #[test]
    fn rss_feed_parses_items_with_titles_and_relative_links() {
        let entries = parse_feed_content(rss_feed_fixture().as_bytes(), "https://example.com/feed")
            .expect("parses feed");

        assert_eq!(entries.len(), 2);
        let first = &entries[0];
        assert_eq!(
            first.title.as_deref(),
            Some("Relative Link"),
            "title should be decoded"
        );
        assert_eq!(
            first.url.as_deref(),
            Some("https://example.com/relative-post"),
        );
        assert_eq!(
            first.published,
            Some(
                Utc.with_ymd_and_hms(2009, 9, 6, 16, 20, 0)
                    .single()
                    .expect("valid datetime")
            )
        );

        let second = &entries[1];
        assert_eq!(second.title.as_deref(), Some("Guid as URL"));
        assert_eq!(second.url.as_deref(), Some("https://example.com/guid-link"),);
    }

    #[test]
    fn atom_feed_parses_items() {
        let atom = r#"<?xml version="1.0" encoding="utf-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
            <title>Atom</title>
            <entry>
                <title>Atom Item</title>
                <link href="https://example.com/atom" rel="alternate"/>
                <id>atom-123</id>
            </entry>
        </feed>"#;

        let entries = parse_feed_content(atom.as_bytes(), "https://example.com/atom-feed").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url.as_deref(), Some("https://example.com/atom"));
    }

    #[test]
    fn json_feed_parses_items() {
        let json = r#"{
            "version": "https://jsonfeed.org/version/1.1",
            "title": "JSON Feed",
            "items": [
                {
                    "id": "json-1",
                    "url": "https://example.com/json-post",
                    "title": "JSON Item"
                }
            ]
        }"#;

        let entries = parse_feed_content(json.as_bytes(), "https://example.com/json-feed").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].url.as_deref(),
            Some("https://example.com/json-post")
        );
    }

    #[test]
    fn feed_without_items_returns_empty() {
        let empty = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Empty</title><link>https://example.com/</link><description>Empty feed</description></channel></rss>"#;
        let entries = parse_feed_content(empty.as_bytes(), "https://example.com/empty").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn malformed_feed_returns_parse_error() {
        let truncated = b"<?xml version=\"1.0\"?><rss><channel><item></rss>";
        let err = parse_feed_content(truncated, "https://example.com/truncated").unwrap_err();
        match err {
            FeedParseError::ParseFailed { reason } => {
                assert!(
                    reason.contains("unable to parse feed"),
                    "expected parser complaint, got {}",
                    reason
                );
            }
        }
    }

    #[test]
    fn feed_entry_without_link_yields_none_poll_item() {
        let entry = FeedEntry {
            guid: "no-link".to_string(),
            url: None,
            title: None,
            published: None,
        };

        assert!(entry.into_poll_item().is_none());
    }

    #[test]
    fn relative_guid_as_permalink_used_if_no_link() {
        let feed = r#"<?xml version="1.0"?><rss version="2.0"><channel>
            <title>Only GUID</title>
            <link>https://example.com/</link>
            <description>Test</description>
            <item><title>Only Guid</title><guid>https://example.com/guid-only</guid></item>
        </channel></rss>"#;

        let entries = parse_feed_content(feed.as_bytes(), "https://example.com/only-guid").unwrap();
        assert_eq!(
            entries[0].url.as_deref(),
            Some("https://example.com/guid-only")
        );
    }
}
