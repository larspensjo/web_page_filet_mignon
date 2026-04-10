/// Pure text formatting utilities, shared across render modules.
pub(super) const MAX_VIEWER_CHARS: usize = 64 * 1024;

pub(super) fn format_compact_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        n.to_string()
    }
}

pub(super) fn compact_url_label(url: &str, max_chars: usize) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return "(untitled source)".to_string();
    }

    let without_scheme = trimmed
        .find("://")
        .map(|pos| &trimmed[pos + 3..])
        .unwrap_or(trimmed);
    let without_query = without_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .trim_end_matches('/');
    let mut segments = without_query
        .split('/')
        .filter(|segment| !segment.is_empty());
    let Some(host) = segments.next() else {
        return truncate_with_ellipsis(trimmed, max_chars);
    };
    let path_segments: Vec<&str> = segments.collect();
    let compact = match path_segments.as_slice() {
        [] => host.to_string(),
        [only] => format!("{host}/{only}"),
        [first, second] => format!("{host}/{first}/{second}"),
        [first, .., last] => format!("{host}/{first}/.../{last}"),
    };
    truncate_with_ellipsis(&compact, max_chars)
}

pub(super) fn compact_triage_tag_count(tags: &[String]) -> Option<String> {
    if tags.is_empty() {
        return None;
    }
    if tags.len() == 1 {
        Some("1 tag".to_string())
    } else {
        Some(format!("{} tags", tags.len()))
    }
}

pub(super) fn title_case_label(value: &str) -> String {
    let mut out = Vec::new();
    for word in value.split(['-', '_', ' ']).filter(|word| !word.is_empty()) {
        let mut chars = word.chars();
        let Some(first) = chars.next() else {
            continue;
        };
        let rest: String = chars.collect();
        out.push(format!(
            "{}{}",
            first.to_uppercase(),
            rest.to_ascii_lowercase()
        ));
    }
    if out.is_empty() {
        value.trim().to_string()
    } else {
        out.join(" ")
    }
}

pub(super) fn domain_from_url(url: &str) -> String {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .find("://")
        .map(|pos| &trimmed[pos + 3..])
        .unwrap_or(trimmed);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .trim_end_matches('/');
    if host.is_empty() {
        trimmed.to_string()
    } else {
        host.to_string()
    }
}

pub(super) fn truncate_with_ellipsis(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    let char_count = trimmed.chars().count();
    if char_count <= max_chars {
        return trimmed.to_string();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }

    let prefix: String = trimmed.chars().take(max_chars - 3).collect();
    format!("{prefix}...")
}

pub(super) fn format_compact_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;

    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

pub(super) fn strip_leading_h1(text: &str) -> &str {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix("# ") {
        let end = rest.find('\n').map_or(rest.len(), |i| i + 1);
        rest[end..].trim_start_matches('\n')
    } else {
        trimmed
    }
}

pub(super) fn truncate_markdown_for_preview(text: &str) -> (String, bool) {
    let total_chars = text.chars().count();
    if total_chars <= MAX_VIEWER_CHARS {
        return (text.to_string(), false);
    }

    let cutoff = text
        .char_indices()
        .nth(MAX_VIEWER_CHARS)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    (text[..cutoff].to_string(), true)
}
