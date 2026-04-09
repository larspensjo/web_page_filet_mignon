use super::{normalize_url_for_dedupe, JobId};
use std::collections::HashSet;
use url::Url;

const INDIRECT_LINK_BLOCKED_HOSTS: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "facebook.com",
    "twitter.com",
    "x.com",
    "linkedin.com",
    "instagram.com",
    "whatsapp.com",
    "pinterest.com",
    "flipboard.com",
    "tiktok.com",
    "zdcs.link",
];
const INDIRECT_LINK_BLOCKED_PATH_PREFIXES: &[&str] = &[
    "/about",
    "/author",
    "/authors",
    "/category",
    "/categories",
    "/contact",
    "/creators",
    "/login",
    "/new",
    "/privacy",
    "/search",
    "/subscription",
    "/tag",
    "/tags",
    "/terms",
];
const INDIRECT_LINK_BLOCKED_PATH_CONTAINS: &[&str] = &[
    "/share",
    "/sharer",
    "/intent/",
    "/intent?",
    "/bookmark",
    "/contact_us",
    "/about/press",
    "/about/copyright",
    "/about/policies",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndirectLink {
    pub(super) url: String,
    pub(super) source_job_id: JobId,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct IndirectLinkPool {
    generation: u32,
    links: Vec<IndirectLink>,
    seen_urls: HashSet<String>,
}

impl IndirectLinkPool {
    pub(super) fn new() -> Self {
        Self {
            generation: 0,
            links: Vec::new(),
            seen_urls: HashSet::new(),
        }
    }

    pub(super) fn begin_new_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.links.clear();
        self.seen_urls.clear();
    }

    pub(super) fn len(&self) -> usize {
        self.links.len()
    }

    pub(super) fn generation(&self) -> u32 {
        self.generation
    }

    pub(super) fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    pub(super) fn draining_links(&mut self) -> Vec<IndirectLink> {
        let drained = self.links.drain(..).collect();
        self.seen_urls.clear();
        drained
    }

    pub(super) fn add_link(&mut self, link: IndirectLink) -> bool {
        let normalized = normalize_url_for_dedupe(&link.url);
        if normalized.is_empty() || self.seen_urls.contains(&normalized) {
            return false;
        }
        self.seen_urls.insert(normalized);
        self.links.push(link);
        true
    }
}

fn host_matches_indirect_blocklist(host: &str) -> bool {
    INDIRECT_LINK_BLOCKED_HOSTS
        .iter()
        .any(|blocked| host == *blocked || host.ends_with(&format!(".{blocked}")))
}

pub(super) fn should_collect_indirect_link(source_url: &str, link_url: &str) -> bool {
    let link = match Url::parse(link_url) {
        Ok(url) => url,
        Err(_) => return false,
    };

    match link.scheme() {
        "http" | "https" => {}
        _ => return false,
    }

    let host = match link.host_str() {
        Some(host) => host.to_ascii_lowercase(),
        None => return false,
    };
    if host_matches_indirect_blocklist(&host) {
        return false;
    }

    let path = link.path().to_ascii_lowercase();
    if path == "/" {
        return false;
    }
    if INDIRECT_LINK_BLOCKED_PATH_PREFIXES
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
    {
        return false;
    }
    if INDIRECT_LINK_BLOCKED_PATH_CONTAINS
        .iter()
        .any(|pattern| path.contains(pattern))
    {
        return false;
    }

    if let Some(query) = link.query() {
        let lower = query.to_ascii_lowercase();
        if lower.contains("utm_")
            || lower.contains("fbclid=")
            || lower.contains("gclid=")
            || lower.contains("redirect=")
            || lower.contains("share=")
            || lower.contains("intent=")
        {
            return false;
        }
    }

    if let Ok(source) = Url::parse(source_url) {
        if source.host_str().map(|h| h.eq_ignore_ascii_case(&host)) == Some(true) {
            let articleish_markers = [
                "/20",
                "/article/",
                "/articles/",
                "/story/",
                "/news/",
                "/insights/",
                "/p/",
            ];
            let looks_articleish = articleish_markers
                .iter()
                .any(|marker| path.contains(marker));
            if !looks_articleish {
                return false;
            }
        }
    }

    true
}
